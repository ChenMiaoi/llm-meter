use std::{
    collections::{BTreeMap, HashMap},
    process::Stdio,
    time::Duration,
};

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use llm_meter_core::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
};
use uuid::Uuid;

pub struct SubscriptionAdapter {
    command: String,
    login: Mutex<HashMap<String, AppServer>>,
}
impl Default for SubscriptionAdapter {
    fn default() -> Self {
        Self {
            command: "codex".into(),
            login: Mutex::new(HashMap::new()),
        }
    }
}
impl SubscriptionAdapter {
    pub fn with_command(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            login: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl ProviderAdapter for SubscriptionAdapter {
    fn manifest(&self) -> ProviderManifest {
        crate::manifest()
    }
    fn supported_auth_schemes(&self) -> Vec<AuthScheme> {
        vec![AuthScheme::OAuthBrowser, AuthScheme::OAuthDeviceCode]
    }
    async fn begin_auth(&self, r: BeginAuthRequest) -> Result<AuthChallenge, ProviderError> {
        let mut server = AppServer::spawn(&self.command).await?;
        let params = match r.auth_scheme {
            AuthScheme::OAuthBrowser => json!({"type":"chatgpt","useHostedLoginSuccessPage":true}),
            AuthScheme::OAuthDeviceCode => json!({"type":"chatgptDeviceCode"}),
            _ => return Err(ProviderError::CapabilityUnavailable),
        };
        let response: LoginResponse =
            serde_json::from_value(server.request("account/login/start", params).await?)
                .map_err(|_| ProviderError::InvalidResponse)?;
        let challenge = match response {
            LoginResponse::Chatgpt { auth_url, login_id } => AuthChallenge::Browser {
                auth_url,
                state: login_id,
            },
            LoginResponse::Device {
                verification_url,
                user_code,
                login_id,
            } => AuthChallenge::DeviceCode {
                verification_url,
                user_code,
                state: login_id,
                expires_at: Utc::now() + chrono::Duration::minutes(15),
                interval_seconds: 5,
            },
            _ => return Err(ProviderError::InvalidResponse),
        };
        let state = match &challenge {
            AuthChallenge::Browser { state, .. } | AuthChallenge::DeviceCode { state, .. } => {
                state.clone()
            }
            _ => unreachable!(),
        };
        self.login.lock().await.insert(state, server);
        Ok(challenge)
    }
    async fn complete_auth(
        &self,
        request: CompleteAuthRequest,
        _: &dyn SecretStore,
    ) -> Result<ConnectionIdentity, ProviderError> {
        let state = request
            .challenge_state
            .ok_or(ProviderError::AuthenticationRequired)?;
        let mut server = self
            .login
            .lock()
            .await
            .remove(&state)
            .ok_or(ProviderError::AuthenticationRequired)?;
        server.wait_login().await?;
        let account: AccountResponse = server
            .typed("account/read", json!({"refreshToken":false}))
            .await?;
        identity(account)
    }
    async fn probe_capabilities(
        &self,
        c: &ConnectionContext,
    ) -> Result<CapabilitySnapshot, ProviderError> {
        let mut server = AppServer::spawn(&self.command).await?;
        let account: AccountResponse = server
            .typed("account/read", json!({"refreshToken":false}))
            .await?;
        if account.account.is_none() {
            return Err(ProviderError::AuthenticationRequired);
        }
        let mut caps = Capabilities::ACCOUNT_INFO | Capabilities::PLAN_INFO;
        if server
            .request("account/rateLimits/read", Value::Null)
            .await
            .is_ok()
        {
            caps |= Capabilities::QUOTA_WINDOWS | Capabilities::CREDITS_BALANCE;
        }
        if server
            .request("account/usage/read", Value::Null)
            .await
            .is_ok()
        {
            caps |= Capabilities::TOKEN_TOTAL | Capabilities::TOKEN_DAILY;
        }
        Ok(CapabilitySnapshot {
            connection_id: c.connection.id,
            capabilities: caps,
            observed_at: Utc::now(),
        })
    }
    async fn sync(
        &self,
        c: &ConnectionContext,
        _: Option<SyncCursor>,
    ) -> Result<SyncBatch, ProviderError> {
        let mut server = AppServer::spawn(&self.command).await?;
        let observed = Utc::now();
        let account: AccountResponse = server
            .typed("account/read", json!({"refreshToken":false}))
            .await?;
        let ident = identity(account.clone())?;
        let rates: RateLimitsResponse =
            server.typed("account/rateLimits/read", Value::Null).await?;
        let usage: UsageResponse = server.typed("account/usage/read", Value::Null).await?;
        let mut quotas = Vec::new();
        let buckets = rates.rate_limits_by_limit_id.unwrap_or_else(|| {
            let id = rates
                .rate_limits
                .limit_id
                .clone()
                .unwrap_or_else(|| "default".into());
            HashMap::from([(id, rates.rate_limits.clone())])
        });
        for (id, s) in buckets {
            for (suffix, w) in [("primary", s.primary), ("secondary", s.secondary)] {
                if let Some(w) = w {
                    let used = Decimal::from(w.used_percent) / Decimal::ONE_HUNDRED;
                    quotas.push(QuotaWindow {
                        id: Uuid::new_v4(),
                        connection_id: c.connection.id,
                        provider_limit_id: format!("{id}:{suffix}"),
                        display_name: Some(quota_display_name(
                            &id,
                            s.limit_name.as_deref(),
                            suffix,
                            w.window_duration_mins,
                        )),
                        window_kind: WindowKind::Rolling,
                        window_start: w
                            .window_duration_mins
                            .map(|m| observed - chrono::Duration::minutes(m)),
                        window_end: w.resets_at.and_then(timestamp),
                        resets_at: w.resets_at.and_then(timestamp),
                        used_ratio: Some(used),
                        remaining_ratio: Some(Decimal::ONE - used),
                        used_value: None,
                        limit_value: None,
                        unit: Some(MetricUnit::Ratio),
                        provenance: Provenance::ProviderReported,
                        observed_at: observed,
                    });
                }
            }
        }
        let mut metrics = Vec::new();
        if let Some(v) = usage.summary.lifetime_tokens {
            metrics.push(sample(
                c.connection.id,
                MetricKey::TOKEN_TOTAL,
                Decimal::from(v),
                None,
                None,
                observed,
                "account.usage.summary.lifetimeTokens",
            ));
        }
        for b in usage.daily_usage_buckets.unwrap_or_default() {
            if let Ok(day) = NaiveDate::parse_from_str(&b.start_date, "%Y-%m-%d") {
                let start = day.and_hms_opt(0, 0, 0).unwrap().and_utc();
                metrics.push(sample(
                    c.connection.id,
                    MetricKey::TOKEN_TOTAL,
                    Decimal::from(b.tokens),
                    Some(start),
                    Some(start + chrono::Duration::days(1)),
                    observed,
                    "account.usage.dailyUsageBuckets.tokens",
                ));
            }
        }
        if let Some(credits) = rates
            .rate_limits
            .credits
            .and_then(|v| v.balance)
            .and_then(|v| v.parse::<Decimal>().ok())
        {
            metrics.push(sample(
                c.connection.id,
                MetricKey::CREDIT_BALANCE,
                credits,
                None,
                None,
                observed,
                "account.rateLimits.credits.balance",
            ));
        }
        let reset_credits = rates
            .rate_limit_reset_credits
            .map(|summary| RateLimitResetCredits {
                connection_id: c.connection.id,
                available_count: summary.available_count,
                credits: summary.credits.map(|credits| {
                    credits
                        .into_iter()
                        .filter_map(|credit| {
                            Some(RateLimitResetCredit {
                                id: credit.id,
                                connection_id: c.connection.id,
                                reset_type: credit.reset_type,
                                status: credit.status,
                                granted_at: timestamp(credit.granted_at)?,
                                expires_at: credit.expires_at.and_then(timestamp),
                                title: credit.title,
                                description: credit.description,
                            })
                        })
                        .collect()
                }),
                observed_at: observed,
            });
        let account_record = AccountRecord {
            id: Uuid::new_v4(),
            connection_id: c.connection.id,
            external_id: ident.external_id.clone(),
            display_name: ident.display_name.clone(),
            account_type: Some("chatgpt".into()),
        };
        let plan = account.account.and_then(|a| a.plan_type);
        Ok(SyncBatch {
            account_updates: vec![account_record],
            product_updates: vec![ProductRecord {
                id: Uuid::new_v4(),
                connection_id: c.connection.id,
                product_key: "chatgpt_codex".into(),
                display_name: plan
                    .map(|value| format!("ChatGPT {value}"))
                    .or_else(|| Some("套餐未提供".into())),
            }],
            capability_snapshot: Some(CapabilitySnapshot {
                connection_id: c.connection.id,
                capabilities: Capabilities::ACCOUNT_INFO
                    | Capabilities::PLAN_INFO
                    | Capabilities::QUOTA_WINDOWS
                    | Capabilities::TOKEN_TOTAL
                    | Capabilities::TOKEN_DAILY
                    | Capabilities::CREDITS_BALANCE,
                observed_at: observed,
            }),
            metric_samples: metrics,
            quota_windows: quotas,
            rate_limit_reset_credits: reset_credits,
            usage_events: Vec::new(),
            provider_events: Vec::new(),
            next_cursor: None,
            provider_timestamp: Some(observed),
        })
    }
    async fn disconnect(
        &self,
        _: &ConnectionContext,
        _: &dyn SecretStore,
    ) -> Result<(), ProviderError> {
        let mut s = AppServer::spawn(&self.command).await?;
        s.request("account/logout", Value::Null).await?;
        Ok(())
    }
}

struct AppServer {
    _child: Child,
    input: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    id: u64,
}
impl AppServer {
    async fn spawn(command: &str) -> Result<Self, ProviderError> {
        let mut child = Command::new(command)
            .arg("app-server")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| ProviderError::UnsupportedVersion)?;
        let input = child
            .stdin
            .take()
            .ok_or(ProviderError::Internal("app-server stdin".into()))?;
        let output = child
            .stdout
            .take()
            .ok_or(ProviderError::Internal("app-server stdout".into()))?;
        let mut s = Self {
            _child: child,
            input,
            lines: BufReader::new(output).lines(),
            id: 0,
        };
        s.request("initialize",json!({"clientInfo":{"name":"llm_meter","title":"LLM Meter","version":env!("CARGO_PKG_VERSION")}})).await?;
        s.notify("initialized", json!({})).await?;
        Ok(s)
    }
    async fn send(&mut self, v: Value) -> Result<(), ProviderError> {
        let mut line = serde_json::to_vec(&v)
            .map_err(|_| ProviderError::Internal("serialize request".into()))?;
        line.push(b'\n');
        self.input
            .write_all(&line)
            .await
            .map_err(|_| ProviderError::NetworkUnavailable)
    }
    async fn notify(&mut self, method: &str, params: Value) -> Result<(), ProviderError> {
        self.send(json!({"method":method,"params":params})).await
    }
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, ProviderError> {
        self.id += 1;
        let id = self.id;
        self.send(json!({"id":id,"method":method,"params":params}))
            .await?;
        loop {
            let line = tokio::time::timeout(Duration::from_secs(30), self.lines.next_line())
                .await
                .map_err(|_| ProviderError::Timeout)?
                .map_err(|_| ProviderError::NetworkUnavailable)?
                .ok_or(ProviderError::NetworkUnavailable)?;
            let v: Value =
                serde_json::from_str(&line).map_err(|_| ProviderError::InvalidResponse)?;
            if v.get("id").and_then(Value::as_u64) == Some(id) {
                if v.get("error").is_some() {
                    return Err(ProviderError::InvalidResponse);
                }
                return Ok(v.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }
    async fn typed<T: for<'de> Deserialize<'de>>(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<T, ProviderError> {
        serde_json::from_value(self.request(method, params).await?)
            .map_err(|_| ProviderError::InvalidResponse)
    }
    async fn wait_login(&mut self) -> Result<(), ProviderError> {
        loop {
            let line = tokio::time::timeout(Duration::from_secs(900), self.lines.next_line())
                .await
                .map_err(|_| ProviderError::Timeout)?
                .map_err(|_| ProviderError::NetworkUnavailable)?
                .ok_or(ProviderError::NetworkUnavailable)?;
            let v: Value =
                serde_json::from_str(&line).map_err(|_| ProviderError::InvalidResponse)?;
            if v.get("method").and_then(Value::as_str) == Some("account/login/completed") {
                return if v.pointer("/params/success").and_then(Value::as_bool) == Some(true) {
                    Ok(())
                } else {
                    Err(ProviderError::AuthenticationRequired)
                };
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
enum LoginResponse {
    #[serde(rename = "chatgpt")]
    Chatgpt {
        #[serde(rename = "authUrl")]
        auth_url: String,
        #[serde(rename = "loginId")]
        login_id: String,
    },
    #[serde(rename = "chatgptDeviceCode")]
    Device {
        #[serde(rename = "verificationUrl")]
        verification_url: String,
        #[serde(rename = "userCode")]
        user_code: String,
        #[serde(rename = "loginId")]
        login_id: String,
    },
    #[serde(other)]
    Other,
}
#[derive(Debug, Clone, Deserialize)]
struct AccountResponse {
    account: Option<Account>,
    #[serde(rename = "requiresOpenaiAuth")]
    _requires_auth: bool,
}
#[derive(Debug, Clone, Deserialize)]
struct Account {
    email: Option<String>,
    #[serde(rename = "planType")]
    plan_type: Option<String>,
}
#[derive(Debug, Clone, Deserialize, Default)]
struct RateLimitsResponse {
    #[serde(rename = "rateLimits")]
    rate_limits: RateSnapshot,
    #[serde(rename = "rateLimitsByLimitId")]
    rate_limits_by_limit_id: Option<HashMap<String, RateSnapshot>>,
    #[serde(rename = "rateLimitResetCredits")]
    rate_limit_reset_credits: Option<ResetCreditsResponse>,
}
#[derive(Debug, Clone, Deserialize, Default)]
struct RateSnapshot {
    #[serde(rename = "limitId")]
    limit_id: Option<String>,
    #[serde(rename = "limitName")]
    limit_name: Option<String>,
    primary: Option<RateWindow>,
    secondary: Option<RateWindow>,
    credits: Option<Credits>,
}
#[derive(Debug, Clone, Deserialize)]
struct RateWindow {
    #[serde(rename = "usedPercent")]
    used_percent: i64,
    #[serde(rename = "resetsAt")]
    resets_at: Option<i64>,
    #[serde(rename = "windowDurationMins")]
    window_duration_mins: Option<i64>,
}
#[derive(Debug, Clone, Deserialize)]
struct Credits {
    balance: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
struct ResetCreditsResponse {
    #[serde(rename = "availableCount")]
    available_count: u64,
    credits: Option<Vec<ResetCreditResponse>>,
}
#[derive(Debug, Clone, Deserialize)]
struct ResetCreditResponse {
    id: String,
    #[serde(rename = "resetType")]
    reset_type: String,
    status: String,
    #[serde(rename = "grantedAt")]
    granted_at: i64,
    #[serde(rename = "expiresAt")]
    expires_at: Option<i64>,
    title: Option<String>,
    description: Option<String>,
}
#[derive(Debug, Clone, Deserialize, Default)]
struct UsageResponse {
    summary: UsageSummary,
    #[serde(rename = "dailyUsageBuckets")]
    daily_usage_buckets: Option<Vec<DailyBucket>>,
}
#[derive(Debug, Clone, Deserialize, Default)]
struct UsageSummary {
    #[serde(rename = "lifetimeTokens")]
    lifetime_tokens: Option<i64>,
}
#[derive(Debug, Clone, Deserialize)]
struct DailyBucket {
    #[serde(rename = "startDate")]
    start_date: String,
    tokens: i64,
}
fn identity(a: AccountResponse) -> Result<ConnectionIdentity, ProviderError> {
    let a = a.account.ok_or(ProviderError::AuthenticationRequired)?;
    let external = a.email.clone().unwrap_or_else(|| "chatgpt-account".into());
    Ok(ConnectionIdentity {
        external_id: external,
        display_name: a.email,
        credential_ref: None,
        settings: None,
    })
}
fn timestamp(v: i64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp(v, 0)
}
fn quota_display_name(
    id: &str,
    limit_name: Option<&str>,
    suffix: &str,
    window_minutes: Option<i64>,
) -> String {
    if id == "codex" && window_minutes == Some(7 * 24 * 60) {
        return "GPT 周额度".into();
    }
    let base = limit_name.unwrap_or(id);
    if suffix == "primary" {
        base.to_owned()
    } else {
        format!("{base} {suffix}")
    }
}
fn sample(
    id: Uuid,
    key: &str,
    value: Decimal,
    start: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
    observed: DateTime<Utc>,
    source: &str,
) -> MetricSample {
    let mut m = MetricSample {
        id: Uuid::new_v4(),
        connection_id: id,
        metric_key: MetricKey(key.into()),
        value,
        unit: MetricUnit::Token,
        scope: MetricScope::Account,
        period_start: start,
        period_end: end,
        observed_at: observed,
        provenance: Provenance::ProviderReported,
        dimensions: BTreeMap::new(),
        source_metric: source.into(),
        dedup_key: String::new(),
    };
    m.dedup_key = m.compute_dedup_key();
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_multi_bucket_schema_is_tolerant() {
        let value = serde_json::json!({"rateLimits":{"limitId":"legacy","primary":{"usedPercent":12,"future":1}},"rateLimitsByLimitId":{"codex":{"limitId":"codex","limitName":"Codex","planType":"pro","primary":{"usedPercent":37,"resetsAt":1900000000,"windowDurationMins":300},"secondary":{"usedPercent":19,"resetsAt":1900100000,"windowDurationMins":10080},"unknown":true}},"rateLimitResetCredits":{"availableCount":1,"credits":[{"id":"credit-1","resetType":"codexRateLimits","status":"available","grantedAt":1890000000,"expiresAt":1900000000,"title":"Full reset","description":null}]}});
        let parsed: RateLimitsResponse = serde_json::from_value(value).unwrap();
        let reset_credits = parsed.rate_limit_reset_credits.as_ref().unwrap();
        assert_eq!(reset_credits.available_count, 1);
        assert_eq!(reset_credits.credits.as_ref().unwrap()[0].id, "credit-1");
        assert_eq!(
            parsed.rate_limits_by_limit_id.unwrap()["codex"]
                .primary
                .as_ref()
                .unwrap()
                .used_percent,
            37
        );
    }
    #[test]
    fn missing_usage_values_stay_missing() {
        let usage: UsageResponse = serde_json::from_value(
            serde_json::json!({"summary":{"future":true},"dailyUsageBuckets":null}),
        )
        .unwrap();
        assert_eq!(usage.summary.lifetime_tokens, None);
    }
    #[test]
    fn codex_seven_day_window_is_named_weekly_quota() {
        assert_eq!(
            quota_display_name("codex", None, "primary", Some(10080)),
            "GPT 周额度"
        );
    }
}
