use std::{
    collections::HashMap,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use llm_meter_core::*;
use reqwest::{
    Method,
    header::{self, HeaderMap, HeaderValue},
};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::json;
use tokio::sync::Mutex;
use uuid::Uuid;

const CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
const DEFAULT_OAUTH_HOST: &str = "https://auth.kimi.com";
const DEFAULT_BASE_URL: &str = "https://api.kimi.com/coding/v1";
const USAGE_PATH: &str = "usages";
const DEVICE_ID_FILENAME: &str = "kimi-device-id";
const OAUTH_EXPIRY_SKEW_SECONDS: i64 = 300;
const DEFAULT_POLL_INTERVAL_SECONDS: u64 = 5;
const DEFAULT_DEVICE_FLOW_TTL_SECONDS: i64 = 15 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KimiCredentials {
    access_token: String,
    refresh_token: String,
    expires_at: DateTime<Utc>,
}

impl KimiCredentials {
    fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at - chrono::Duration::seconds(OAUTH_EXPIRY_SKEW_SECONDS)
    }
}

struct PendingDevice {
    display_name: String,
    interval_seconds: u64,
    expires_at: DateTime<Utc>,
}

pub struct SubscriptionAdapter {
    oauth_host: String,
    base_url: String,
    http_client: reqwest::Client,
    pending: Mutex<HashMap<String, PendingDevice>>,
    device_id: String,
}

impl Default for SubscriptionAdapter {
    fn default() -> Self {
        Self::new(
            DEFAULT_OAUTH_HOST.into(),
            DEFAULT_BASE_URL.into(),
            default_device_id_path(),
        )
    }
}

impl SubscriptionAdapter {
    pub fn new(oauth_host: String, base_url: String, device_id_path: PathBuf) -> Self {
        let device_id = load_or_create_device_id(&device_id_path);
        Self {
            oauth_host,
            base_url,
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            pending: Mutex::new(HashMap::new()),
            device_id,
        }
    }

    fn common_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        let version = env!("CARGO_PKG_VERSION");
        let user_agent = format!("KimiCLI/{version}");
        headers.insert(header::USER_AGENT, header_value(&user_agent));
        headers.insert("X-Msh-Platform", HeaderValue::from_static("kimi_cli"));
        headers.insert("X-Msh-Version", header_value(version));
        headers.insert("X-Msh-Device-Name", header_value(&hostname()));
        headers.insert("X-Msh-Device-Model", header_value(&device_model()));
        headers.insert("X-Msh-Os-Version", header_value(&os_version()));
        headers.insert("X-Msh-Device-Id", header_value(&self.device_id));
        headers
    }
}

fn default_device_id_path() -> PathBuf {
    std::env::var_os("LLM_METER_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".llm-meter")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join(DEVICE_ID_FILENAME)
}

fn load_or_create_device_id(path: &Path) -> String {
    if let Ok(existing) = std::fs::read_to_string(path) {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return sanitize_device_id(trimmed);
        }
    }
    let id = Uuid::new_v4().to_string().replace('-', "");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::write(path, format!("{id}\n")).is_ok() {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).ok();
    }
    id
}

fn sanitize_device_id(value: &str) -> String {
    value.replace(['\n', '\r', ' ', '\t'], "")
}

fn header_value(value: &str) -> HeaderValue {
    let sanitized: String = value
        .chars()
        .filter(|c| ('\x20'..='\x7E').contains(c))
        .collect();
    HeaderValue::from_str(&sanitized).unwrap_or_else(|_| HeaderValue::from_static("unknown"))
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown".into())
}

fn device_model() -> String {
    format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
}

fn os_version() -> String {
    "unknown".into()
}

#[async_trait]
impl ProviderAdapter for SubscriptionAdapter {
    fn manifest(&self) -> ProviderManifest {
        crate::manifest()
    }

    fn supported_auth_schemes(&self) -> Vec<AuthScheme> {
        vec![AuthScheme::OAuthDeviceCode]
    }

    async fn begin_auth(&self, r: BeginAuthRequest) -> Result<AuthChallenge, ProviderError> {
        if r.auth_scheme != AuthScheme::OAuthDeviceCode {
            return Err(ProviderError::CapabilityUnavailable);
        }
        let device = self.request_device_authorization().await?;
        let expires_in = chrono::Duration::seconds(device.expires_in_seconds());
        let device_code = device.device_code.clone();
        let user_code = device.user_code.clone();
        let verification_uri = device.verification_uri();
        let interval_seconds = device.interval_seconds();
        let pending = PendingDevice {
            display_name: r.display_name,
            interval_seconds,
            expires_at: Utc::now() + expires_in,
        };
        self.pending
            .lock()
            .await
            .insert(device_code.clone(), pending);
        Ok(AuthChallenge::DeviceCode {
            verification_url: verification_uri,
            user_code,
            state: device_code,
            expires_at: Utc::now() + expires_in,
            interval_seconds,
        })
    }

    async fn complete_auth(
        &self,
        request: CompleteAuthRequest,
        secrets: &dyn SecretStore,
    ) -> Result<ConnectionIdentity, ProviderError> {
        let state = request
            .challenge_state
            .ok_or(ProviderError::AuthenticationRequired)?;
        let pending = self
            .pending
            .lock()
            .await
            .remove(&state)
            .ok_or(ProviderError::AuthenticationRequired)?;
        let credentials = self
            .poll_token(&state, pending.interval_seconds, pending.expires_at)
            .await?;
        let credential_ref = store_credentials(secrets, &credentials).await?;
        Ok(ConnectionIdentity {
            external_id: "kimi-code".into(),
            display_name: Some(pending.display_name),
            credential_ref: Some(credential_ref),
        })
    }

    async fn refresh_credentials(
        &self,
        context: &ConnectionContext,
        _secrets: &dyn SecretStore,
    ) -> Result<Option<SecretString>, ProviderError> {
        let secret = match &context.auth_secret {
            Some(s) => s,
            None => return Ok(None),
        };
        let credentials: KimiCredentials = serde_json::from_str(secret.expose_secret())
            .map_err(|_| ProviderError::InvalidResponse)?;
        if !credentials.is_expired() {
            return Ok(None);
        }
        let refreshed = self
            .refresh_access_token(&credentials.refresh_token)
            .await?;
        let secret_json = serde_json::to_string(&refreshed)
            .map_err(|_| ProviderError::Internal("failed to serialize credentials".into()))?;
        Ok(Some(SecretString::from(secret_json)))
    }

    async fn probe_capabilities(
        &self,
        c: &ConnectionContext,
    ) -> Result<CapabilitySnapshot, ProviderError> {
        let credentials = load_credentials(c)?;
        let url = build_usage_url(&self.base_url);
        match self.fetch_usage(&credentials.access_token, &url).await {
            Ok(_) => Ok(CapabilitySnapshot {
                connection_id: c.connection.id,
                capabilities: Capabilities::ACCOUNT_INFO
                    | Capabilities::PLAN_INFO
                    | Capabilities::QUOTA_WINDOWS,
                observed_at: Utc::now(),
            }),
            Err(ProviderError::AuthenticationRequired) => Ok(CapabilitySnapshot {
                connection_id: c.connection.id,
                capabilities: Capabilities::ACCOUNT_INFO | Capabilities::PLAN_INFO,
                observed_at: Utc::now(),
            }),
            Err(e) => Err(e),
        }
    }

    async fn sync(
        &self,
        c: &ConnectionContext,
        _: Option<SyncCursor>,
    ) -> Result<SyncBatch, ProviderError> {
        let credentials = load_credentials(c)?;
        let url = build_usage_url(&self.base_url);
        let usage: UsageResponse = self.fetch_usage(&credentials.access_token, &url).await?;
        let observed = Utc::now();

        let mut metrics = Vec::new();
        let mut quota_windows = Vec::new();

        if let Some(usage_row) = usage.usage.as_ref() {
            if let Some(used) = usage_row.used {
                metrics.push(sample(
                    c.connection.id,
                    MetricKey::TOKEN_TOTAL,
                    Decimal::from(used),
                    None,
                    None,
                    observed,
                    "usage.used",
                ));
            }
            if let Some(window) =
                build_quota_window("total", "Total quota", usage_row, c.connection.id, observed)
            {
                quota_windows.push(window);
            }
        }

        for (idx, item) in usage.limits.unwrap_or_default().iter().enumerate() {
            let label = item
                .name
                .clone()
                .or_else(|| item.title.clone())
                .or_else(|| item.scope.clone())
                .unwrap_or_else(|| format!("Limit #{}", idx + 1));
            let id = format!("limit-{idx}");
            let detail = item.detail.clone().unwrap_or_default();
            if let Some(window) =
                build_quota_window(&id, &label, &detail, c.connection.id, observed)
            {
                quota_windows.push(window);
            }
        }

        Ok(SyncBatch {
            account_updates: vec![AccountRecord {
                id: Uuid::new_v4(),
                connection_id: c.connection.id,
                external_id: "kimi-code".into(),
                display_name: Some("Kimi Code".into()),
                account_type: Some("kimi_code".into()),
            }],
            product_updates: vec![ProductRecord {
                id: Uuid::new_v4(),
                connection_id: c.connection.id,
                product_key: "kimi_code".into(),
                display_name: Some("Kimi Code".into()),
            }],
            capability_snapshot: Some(CapabilitySnapshot {
                connection_id: c.connection.id,
                capabilities: Capabilities::ACCOUNT_INFO
                    | Capabilities::PLAN_INFO
                    | Capabilities::QUOTA_WINDOWS,
                observed_at: observed,
            }),
            metric_samples: metrics,
            quota_windows,
            rate_limit_reset_credits: None,
            next_cursor: None,
            provider_timestamp: Some(observed),
        })
    }

    async fn disconnect(
        &self,
        c: &ConnectionContext,
        secrets: &dyn SecretStore,
    ) -> Result<(), ProviderError> {
        if let Some(reference) = &c.credential_ref {
            secrets.delete(reference).await?;
        }
        Ok(())
    }
}

async fn store_credentials(
    secrets: &dyn SecretStore,
    credentials: &KimiCredentials,
) -> Result<CredentialRef, ProviderError> {
    let json = serde_json::to_string(credentials)
        .map_err(|_| ProviderError::Internal("failed to serialize credentials".into()))?;
    let key = format!("kimi_code_{}", Uuid::new_v4());
    secrets
        .put("io.github.llmmeter.kimi", &key, SecretString::from(json))
        .await
}

fn load_credentials(c: &ConnectionContext) -> Result<KimiCredentials, ProviderError> {
    let secret = c
        .auth_secret
        .as_ref()
        .ok_or(ProviderError::AuthenticationRequired)?;
    serde_json::from_str(secret.expose_secret()).map_err(|_| ProviderError::InvalidResponse)
}

fn build_usage_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    format!("{base}/{USAGE_PATH}")
}

impl SubscriptionAdapter {
    async fn request_device_authorization(
        &self,
    ) -> Result<DeviceAuthorizationResponse, ProviderError> {
        let url = format!(
            "{}/api/oauth/device_authorization",
            self.oauth_host.trim_end_matches('/')
        );
        let mut headers = self.common_headers();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        let body = format!("client_id={}", CLIENT_ID);
        let response = self
            .http_client
            .request(Method::POST, &url)
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|_| ProviderError::NetworkUnavailable)?;
        let response = map_response(response).await?;
        response
            .json::<DeviceAuthorizationResponse>()
            .await
            .map_err(|_| ProviderError::InvalidResponse)
    }

    async fn poll_token(
        &self,
        device_code: &str,
        interval_seconds: u64,
        expires_at: DateTime<Utc>,
    ) -> Result<KimiCredentials, ProviderError> {
        let url = format!("{}/api/oauth/token", self.oauth_host.trim_end_matches('/'));
        let mut headers = self.common_headers();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        let body = format!(
            "client_id={}&device_code={}&grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code",
            CLIENT_ID, device_code
        );
        let interval = Duration::from_secs(interval_seconds.max(1));
        while Utc::now() < expires_at {
            let response = self
                .http_client
                .request(Method::POST, &url)
                .headers(headers.clone())
                .body(body.clone())
                .send()
                .await
                .map_err(|_| ProviderError::NetworkUnavailable)?;
            let token_response = parse_oauth_token_response(response).await?;
            if let Some(creds) = parse_token_response(token_response, None) {
                return Ok(creds);
            }
            tokio::time::sleep(interval).await;
        }
        Err(ProviderError::Timeout)
    }

    async fn refresh_access_token(
        &self,
        refresh_token: &str,
    ) -> Result<KimiCredentials, ProviderError> {
        let url = format!("{}/api/oauth/token", self.oauth_host.trim_end_matches('/'));
        let mut headers = self.common_headers();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        let body = format!(
            "grant_type=refresh_token&refresh_token={}&client_id={}",
            refresh_token, CLIENT_ID
        );
        let response = self
            .http_client
            .request(Method::POST, &url)
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|_| ProviderError::NetworkUnavailable)?;
        let token_response = parse_oauth_token_response(response).await?;
        parse_token_response(token_response, Some(refresh_token))
            .ok_or(ProviderError::AuthenticationRequired)
    }

    async fn fetch_usage(
        &self,
        access_token: &str,
        url: &str,
    ) -> Result<UsageResponse, ProviderError> {
        let mut headers = self.common_headers();
        headers.insert(
            header::AUTHORIZATION,
            header_value(&format!("Bearer {access_token}")),
        );
        let response = self
            .http_client
            .request(Method::GET, url)
            .headers(headers)
            .send()
            .await
            .map_err(|_| ProviderError::NetworkUnavailable)?;
        let response = map_response(response).await?;
        response
            .json()
            .await
            .map_err(|_| ProviderError::InvalidResponse)
    }
}

async fn parse_oauth_token_response(
    response: reqwest::Response,
) -> Result<TokenResponse, ProviderError> {
    let status = response.status().as_u16();
    let payload: TokenResponse = response
        .json()
        .await
        .map_err(|_| ProviderError::InvalidResponse)?;
    match status {
        200..=299 => {
            if payload.access_token.is_some() {
                Ok(payload)
            } else {
                Err(ProviderError::InvalidResponse)
            }
        }
        _ => match payload.error.as_deref() {
            Some("authorization_pending") => Ok(payload),
            Some("slow_down") => Ok(payload),
            Some("expired_token") => Err(ProviderError::Timeout),
            Some("access_denied") => Err(ProviderError::AuthenticationRequired),
            _ => Err(map_error_status(status)),
        },
    }
}

fn map_error_status(status: u16) -> ProviderError {
    match status {
        401 => ProviderError::AuthenticationRequired,
        403 => ProviderError::PermissionDenied,
        429 => ProviderError::RateLimited { retry_at: None },
        500..=599 => ProviderError::NetworkUnavailable,
        _ => ProviderError::InvalidResponse,
    }
}

fn parse_token_response(
    r: TokenResponse,
    fallback_refresh_token: Option<&str>,
) -> Option<KimiCredentials> {
    let access_token = r.access_token?;
    let refresh_token = r
        .refresh_token
        .or_else(|| fallback_refresh_token.map(str::to_owned))?;
    let expires_in = r.expires_in?;
    Some(KimiCredentials {
        access_token,
        refresh_token,
        expires_at: Utc::now() + chrono::Duration::seconds(expires_in),
    })
}

async fn map_response(response: reqwest::Response) -> Result<reqwest::Response, ProviderError> {
    match response.status().as_u16() {
        200..=299 => Ok(response),
        401 => Err(ProviderError::AuthenticationRequired),
        403 => Err(ProviderError::PermissionDenied),
        429 => Err(ProviderError::RateLimited { retry_at: None }),
        500..=599 => Err(ProviderError::NetworkUnavailable),
        _ => Err(ProviderError::InvalidResponse),
    }
}

#[derive(Debug, Clone, Deserialize)]
struct DeviceAuthorizationResponse {
    user_code: String,
    device_code: String,
    #[serde(alias = "verification_uri")]
    verification_uri: String,
    #[serde(alias = "verification_uri_complete")]
    verification_uri_complete: Option<String>,
    #[serde(alias = "expires_in")]
    expires_in: Option<i64>,
    interval: Option<i64>,
}

impl DeviceAuthorizationResponse {
    fn expires_in_seconds(&self) -> i64 {
        self.expires_in.unwrap_or(DEFAULT_DEVICE_FLOW_TTL_SECONDS)
    }
    fn interval_seconds(&self) -> u64 {
        self.interval
            .map(|i| i.max(1) as u64)
            .unwrap_or(DEFAULT_POLL_INTERVAL_SECONDS)
    }
    fn verification_uri(&self) -> String {
        self.verification_uri_complete
            .clone()
            .unwrap_or_else(|| self.verification_uri.clone())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    error: Option<String>,
    error_description: Option<String>,
    interval: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct UsageResponse {
    usage: Option<UsageRow>,
    limits: Option<Vec<LimitItem>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[allow(dead_code)]
struct UsageRow {
    name: Option<String>,
    title: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_i64")]
    used: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_optional_i64")]
    limit: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_optional_i64")]
    remaining: Option<i64>,
    #[serde(
        default,
        alias = "reset_at",
        alias = "resetAt",
        alias = "reset_time",
        alias = "resetTime",
        deserialize_with = "deserialize_optional_datetime"
    )]
    reset_at: Option<DateTime<Utc>>,
    #[serde(
        default,
        alias = "reset_in",
        alias = "resetIn",
        alias = "ttl",
        alias = "window",
        deserialize_with = "deserialize_optional_i64"
    )]
    reset_in: Option<i64>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum IntegerValue {
    Integer(i64),
    String(String),
}

fn deserialize_optional_i64<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<IntegerValue>::deserialize(deserializer)?;
    value
        .map(|value| match value {
            IntegerValue::Integer(value) => Ok(value),
            IntegerValue::String(value) => value.parse().map_err(serde::de::Error::custom),
        })
        .transpose()
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DateTimeValue {
    Timestamp(i64),
    String(String),
}

fn deserialize_optional_datetime<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<DateTimeValue>::deserialize(deserializer)?;
    value
        .map(|value| match value {
            DateTimeValue::Timestamp(value) => DateTime::from_timestamp(value, 0)
                .ok_or_else(|| serde::de::Error::custom("invalid Unix timestamp")),
            DateTimeValue::String(value) => DateTime::parse_from_rfc3339(&value)
                .map(|value| value.with_timezone(&Utc))
                .map_err(serde::de::Error::custom),
        })
        .transpose()
}

#[derive(Debug, Clone, Deserialize, Default)]
#[allow(dead_code)]
struct LimitItem {
    name: Option<String>,
    title: Option<String>,
    scope: Option<String>,
    detail: Option<UsageRow>,
    window: Option<WindowData>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[allow(dead_code)]
struct WindowData {
    duration: Option<i64>,
    #[serde(alias = "time_unit", alias = "timeUnit")]
    time_unit: Option<String>,
    #[serde(
        default,
        alias = "reset_at",
        alias = "resetAt",
        alias = "reset_time",
        alias = "resetTime",
        deserialize_with = "deserialize_optional_datetime"
    )]
    reset_at: Option<DateTime<Utc>>,
    #[serde(
        default,
        alias = "reset_in",
        alias = "resetIn",
        alias = "ttl",
        deserialize_with = "deserialize_optional_i64"
    )]
    reset_in: Option<i64>,
}

fn build_quota_window(
    id: &str,
    label: &str,
    row: &UsageRow,
    connection_id: Uuid,
    observed: DateTime<Utc>,
) -> Option<QuotaWindow> {
    let limit = row.limit?;
    let used = row
        .used
        .or_else(|| row.remaining.map(|remaining| limit - remaining))?;
    let used_ratio = if limit > 0 {
        (Decimal::from(used) / Decimal::from(limit)).clamp(Decimal::ZERO, Decimal::ONE)
    } else {
        Decimal::ZERO
    };
    Some(QuotaWindow {
        id: Uuid::new_v4(),
        connection_id,
        provider_limit_id: id.into(),
        display_name: Some(label.into()),
        window_kind: WindowKind::Rolling,
        window_start: None,
        window_end: row.reset_at,
        resets_at: row.reset_at,
        used_ratio: Some(used_ratio),
        remaining_ratio: Some(Decimal::ONE - used_ratio),
        used_value: Some(Decimal::from(used)),
        limit_value: Some(Decimal::from(limit)),
        unit: Some(MetricUnit::Token),
        provenance: Provenance::ProviderReported,
        observed_at: observed,
    })
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
        dimensions: std::collections::BTreeMap::new(),
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
    fn parses_usage_response() {
        let value = json!({
            "usage": { "name": "Total", "used": "5", "limit": "100", "remaining": "95", "resetTime": "2026-07-18T08:53:22.310272Z" },
            "limits": [
                { "detail": { "remaining": "100", "limit": "100", "resetTime": "2026-07-14T00:53:22.310272Z" }, "window": { "duration": 300, "timeUnit": "TIME_UNIT_MINUTE" } }
            ]
        });
        let parsed: UsageResponse = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.usage.as_ref().unwrap().used, Some(5));
        assert_eq!(parsed.usage.as_ref().unwrap().limit, Some(100));
        assert_eq!(parsed.limits.as_ref().unwrap().len(), 1);
        assert_eq!(
            parsed.limits.as_ref().unwrap()[0]
                .detail
                .as_ref()
                .unwrap()
                .remaining,
            Some(100)
        );
        let detail = parsed.limits.as_ref().unwrap()[0].detail.as_ref().unwrap();
        let window = build_quota_window(
            "limit-0",
            "5-minute quota",
            detail,
            Uuid::new_v4(),
            Utc::now(),
        )
        .unwrap();
        assert_eq!(window.used_value, Some(Decimal::ZERO));
        assert_eq!(window.limit_value, Some(Decimal::from(100)));
        assert_eq!(
            window.resets_at,
            Some("2026-07-14T00:53:22.310272Z".parse().unwrap())
        );
    }

    #[test]
    fn parses_token_response() {
        let value = json!({
            "access_token": "access-123",
            "refresh_token": "refresh-456",
            "expires_in": 3600
        });
        let parsed: TokenResponse = serde_json::from_value(value).unwrap();
        let creds = parse_token_response(parsed, None).unwrap();
        assert_eq!(creds.access_token, "access-123");
        assert_eq!(creds.refresh_token, "refresh-456");
    }

    #[test]
    fn refresh_response_keeps_existing_refresh_token() {
        let value = json!({
            "access_token": "access-789",
            "expires_in": 3600
        });
        let parsed: TokenResponse = serde_json::from_value(value).unwrap();
        let creds = parse_token_response(parsed, Some("refresh-existing")).unwrap();
        assert_eq!(creds.access_token, "access-789");
        assert_eq!(creds.refresh_token, "refresh-existing");
    }

    #[test]
    fn expired_credentials_flagged() {
        let creds = KimiCredentials {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: Utc::now() - chrono::Duration::seconds(10),
        };
        assert!(creds.is_expired());
    }

    #[test]
    fn builds_quota_window_from_row() {
        let row = UsageRow {
            name: Some("Total".into()),
            title: None,
            used: Some(50),
            limit: Some(100),
            remaining: None,
            reset_at: DateTime::from_timestamp(1890000000, 0),
            reset_in: None,
        };
        let window =
            build_quota_window("total", "Total", &row, Uuid::new_v4(), Utc::now()).unwrap();
        assert_eq!(window.used_value, Some(Decimal::from(50)));
        assert_eq!(window.limit_value, Some(Decimal::from(100)));
    }

    #[test]
    fn clamps_quota_ratios_for_provider_boundary_values() {
        for (used, limit, expected) in [
            (110, 100, Decimal::ONE),
            (-10, 100, Decimal::ZERO),
            (10, 0, Decimal::ZERO),
            (10, -1, Decimal::ZERO),
        ] {
            let row = UsageRow {
                used: Some(used),
                limit: Some(limit),
                ..UsageRow::default()
            };
            let window =
                build_quota_window("total", "Total", &row, Uuid::new_v4(), Utc::now()).unwrap();
            assert_eq!(window.used_ratio, Some(expected));
            assert_eq!(window.remaining_ratio, Some(Decimal::ONE - expected));
            assert_eq!(window.used_value, Some(Decimal::from(used)));
            assert_eq!(window.limit_value, Some(Decimal::from(limit)));
            window.normalize().unwrap();
        }
    }
}
