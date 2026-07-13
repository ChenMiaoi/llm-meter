use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use llm_meter_core::*;
use reqwest::{Client, StatusCode};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, time::Duration};
use url::Url;
use uuid::Uuid;

pub fn manifest() -> ProviderManifest {
    ProviderManifest {
        provider_id: "relay".into(),
        display_name: "OpenAI-Compatible Relay".into(),
        adapter_version: env!("CARGO_PKG_VERSION").into(),
        connection_types: vec![
            connection_type("new_api", "New API Relay"),
            connection_type("openrouter", "OpenRouter"),
            connection_type("openai_compatible_proxy", "Generic OpenAI-Compatible Relay"),
        ],
    }
}

fn connection_type(id: &str, display_name: &str) -> ConnectionTypeManifest {
    ConnectionTypeManifest {
        id: id.into(),
        display_name: display_name.into(),
        auth_schemes: vec![AuthScheme::ApiKey],
    }
}

pub struct RelayAdapter {
    client: Client,
}
impl Default for RelayAdapter {
    fn default() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(concat!("llm-meter/", env!("CARGO_PKG_VERSION")))
                .build()
                .expect("relay HTTP client"),
        }
    }
}

#[derive(Debug, Clone)]
struct RelaySettings {
    profile: String,
    origin: Url,
}

fn settings(
    input: &ConnectionSettings,
    connection_type: &str,
) -> Result<RelaySettings, ProviderError> {
    input
        .validate_public()
        .map_err(|_| ProviderError::InvalidResponse)?;
    if input.schema_version != 1 {
        return Err(ProviderError::UnsupportedVersion);
    }
    let profile = input
        .values
        .get("profile")
        .and_then(Value::as_str)
        .unwrap_or(connection_type)
        .replace('-', "_");
    let expected = match connection_type {
        "new_api" => "new_api",
        "openrouter" => "openrouter",
        "openai_compatible_proxy" => "generic",
        _ => return Err(ProviderError::CapabilityUnavailable),
    };
    if profile != expected {
        return Err(ProviderError::InvalidResponse);
    }
    let origin = input
        .values
        .get("origin")
        .and_then(Value::as_str)
        .or_else(|| (profile == "openrouter").then_some("https://openrouter.ai"))
        .ok_or(ProviderError::InvalidResponse)?;
    validate_origin(origin, input).map(|origin| RelaySettings { profile, origin })
}

fn validate_origin(value: &str, input: &ConnectionSettings) -> Result<Url, ProviderError> {
    let mut url = Url::parse(value).map_err(|_| ProviderError::InvalidResponse)?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_some()
    {
        return Err(ProviderError::InvalidResponse);
    }
    let loopback = url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    let insecure = input
        .values
        .get("allow_insecure_http")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if url.scheme() != "https" && !(url.scheme() == "http" && (loopback || insecure)) {
        return Err(ProviderError::InvalidResponse);
    }
    if url.cannot_be_a_base() {
        return Err(ProviderError::InvalidResponse);
    }
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);
    Ok(url)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CompatibilityReport {
    pub normalized_origin: String,
    pub models: Vec<String>,
}

pub async fn validate_openai_compatible(
    origin: &str,
    secret: &SecretString,
) -> Result<CompatibilityReport, ProviderError> {
    let settings = ConnectionSettings {
        schema_version: 1,
        values: BTreeMap::from([("origin".into(), Value::String(origin.into()))]),
    };
    let origin = validate_origin(origin, &settings)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("llm-meter/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| ProviderError::Internal(error.to_string()))?;
    let response = client
        .get(
            origin
                .join("v1/models")
                .map_err(|_| ProviderError::InvalidResponse)?,
        )
        .bearer_auth(secret.expose_secret())
        .send()
        .await
        .map_err(network_error)?;
    map_status(response.status())?;
    let body: Value = response
        .json()
        .await
        .map_err(|_| ProviderError::InvalidResponse)?;
    let mut models = body
        .get("data")
        .and_then(Value::as_array)
        .ok_or(ProviderError::InvalidResponse)?
        .iter()
        .filter_map(|model| model.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect::<Vec<_>>();
    models.sort();
    models.dedup();
    Ok(CompatibilityReport {
        normalized_origin: origin.to_string().trim_end_matches('/').into(),
        models,
    })
}

impl RelayAdapter {
    async fn get(
        &self,
        config: &RelaySettings,
        path: &str,
        secret: &SecretString,
    ) -> Result<Value, ProviderError> {
        let url = config
            .origin
            .join(path)
            .map_err(|_| ProviderError::InvalidResponse)?;
        let response = self
            .client
            .get(url)
            .bearer_auth(secret.expose_secret())
            .send()
            .await
            .map_err(network_error)?;
        map_status(response.status())?;
        response
            .json()
            .await
            .map_err(|_| ProviderError::InvalidResponse)
    }

    async fn validate(
        &self,
        config: &RelaySettings,
        secret: &SecretString,
    ) -> Result<(), ProviderError> {
        match config.profile.as_str() {
            "new_api" => {
                self.get(config, "/api/usage/token", secret).await?;
            }
            "openrouter" => {
                self.get(config, "/api/v1/activity", secret).await?;
            }
            "generic" => {
                self.get(config, "/v1/models", secret).await?;
            }
            _ => return Err(ProviderError::CapabilityUnavailable),
        }
        Ok(())
    }

    async fn sync_new_api(
        &self,
        context: &ConnectionContext,
        config: &RelaySettings,
        secret: &SecretString,
    ) -> Result<SyncBatch, ProviderError> {
        let observed = Utc::now();
        let usage: TokenUsage = decode_data(self.get(config, "/api/usage/token", secret).await?)?;
        let logs_value = self.get(config, "/api/log/token", secret).await?;
        let logs: Vec<NewApiLog> = decode_list(logs_value)?;
        let events = logs
            .into_iter()
            .filter(|log| log.log_type.is_none_or(|kind| kind == 2 || kind == 0))
            .map(|log| new_api_event(context.connection.id, log, observed))
            .collect::<Result<Vec<_>, _>>()?;
        let used = decimal_value(&usage.total_used);
        let limit = decimal_value(&usage.total_granted);
        let remaining = decimal_value(&usage.total_available);
        let unlimited = usage.unlimited_quota.unwrap_or(false);
        let remaining_ratio = if unlimited {
            None
        } else {
            match (remaining, limit) {
                (Some(r), Some(l)) if l > Decimal::ZERO => Some(r / l),
                _ => None,
            }
        };
        let newest_occurred_at = events_max_time(&events);
        Ok(SyncBatch {
            capability_snapshot: Some(CapabilitySnapshot { connection_id: context.connection.id, capabilities: Capabilities::QUOTA_WINDOWS | Capabilities::TOKEN_INPUT | Capabilities::TOKEN_OUTPUT | Capabilities::TOKEN_TOTAL | Capabilities::REQUEST_COUNT | Capabilities::CREDITS_BALANCE | Capabilities::PER_MODEL | Capabilities::PER_API_KEY, observed_at: observed }),
            usage_events: events,
            quota_windows: vec![QuotaWindow { id: Uuid::new_v4(), connection_id: context.connection.id, provider_limit_id: "new_api.token_quota".into(), display_name: Some(if unlimited { "Unlimited quota".into() } else { "Token quota".into() }), window_kind: WindowKind::Unknown, window_start: None, window_end: None, resets_at: usage.expires_at.and_then(|v| (v > 0).then(|| Utc.timestamp_opt(v, 0).single()).flatten()), used_ratio: None, remaining_ratio, used_value: used, limit_value: limit, unit: Some(MetricUnit::Credit), provenance: Provenance::ProviderReported, observed_at: observed }],
            provider_timestamp: Some(observed),
            next_cursor: Some(SyncCursor(serde_json::json!({"version":1,"newest_occurred_at":newest_occurred_at,"last_cumulative_used":used}).to_string())),
            ..Default::default()
        })
    }

    async fn sync_openrouter(
        &self,
        context: &ConnectionContext,
        config: &RelaySettings,
        secret: &SecretString,
    ) -> Result<SyncBatch, ProviderError> {
        let observed = Utc::now();
        let rows: Vec<OpenRouterActivity> =
            decode_list(self.get(config, "/api/v1/activity", secret).await?)?;
        let events = rows
            .into_iter()
            .map(|row| openrouter_event(context.connection.id, row, observed))
            .collect::<Result<Vec<_>, _>>()?;
        let credits = self
            .get(config, "/api/v1/credits", secret)
            .await
            .ok()
            .and_then(|value| decode_data::<Credits>(value).ok());
        let mut quota_windows = Vec::new();
        if let Some(credits) = credits {
            quota_windows.push(QuotaWindow {
                id: Uuid::new_v4(),
                connection_id: context.connection.id,
                provider_limit_id: "openrouter.credits".into(),
                display_name: Some("OpenRouter credits".into()),
                window_kind: WindowKind::Unknown,
                window_start: None,
                window_end: None,
                resets_at: None,
                used_ratio: None,
                remaining_ratio: None,
                used_value: decimal_value(&credits.total_usage),
                limit_value: decimal_value(&credits.total_credits),
                unit: Some(MetricUnit::Credit),
                provenance: Provenance::ProviderReported,
                observed_at: observed,
            });
        }
        Ok(SyncBatch {
            capability_snapshot: Some(CapabilitySnapshot {
                connection_id: context.connection.id,
                capabilities: Capabilities::TOKEN_INPUT
                    | Capabilities::TOKEN_OUTPUT
                    | Capabilities::TOKEN_REASONING
                    | Capabilities::TOKEN_TOTAL
                    | Capabilities::REQUEST_COUNT
                    | Capabilities::COST_ACTUAL
                    | Capabilities::PER_MODEL
                    | Capabilities::TOKEN_DAILY,
                observed_at: observed,
            }),
            usage_events: events,
            quota_windows,
            provider_timestamp: Some(observed),
            next_cursor: Some(SyncCursor(
                serde_json::json!({"version":1,"overlap_start":observed-chrono::Duration::days(3)})
                    .to_string(),
            )),
            ..Default::default()
        })
    }
}

#[async_trait]
impl ProviderAdapter for RelayAdapter {
    fn manifest(&self) -> ProviderManifest {
        manifest()
    }
    fn supported_auth_schemes(&self) -> Vec<AuthScheme> {
        vec![AuthScheme::ApiKey]
    }
    async fn begin_auth(&self, request: BeginAuthRequest) -> Result<AuthChallenge, ProviderError> {
        settings(&request.settings, &request.connection_type)?;
        if request.auth_scheme != AuthScheme::ApiKey {
            return Err(ProviderError::CapabilityUnavailable);
        }
        Ok(AuthChallenge::SecretInput {
            challenge_id: String::new(),
            label: "Relay API Key".into(),
            placeholder: Some("sk-…".into()),
        })
    }
    async fn complete_auth(
        &self,
        request: CompleteAuthRequest,
        secrets: &dyn SecretStore,
    ) -> Result<ConnectionIdentity, ProviderError> {
        let secret = request
            .secret
            .ok_or(ProviderError::AuthenticationRequired)?;
        let connection_type = request
            .connection_type
            .ok_or_else(|| ProviderError::Internal("missing relay connection type".into()))?;
        let pending_settings = request
            .settings
            .ok_or_else(|| ProviderError::Internal("missing relay settings".into()))?;
        let config = settings(&pending_settings, &connection_type)?;
        self.validate(&config, &secret).await?;
        let reference = secrets
            .put(
                "io.github.llmmeter.relay",
                &format!("connection_{}", Uuid::new_v4()),
                secret,
            )
            .await?;
        Ok(ConnectionIdentity {
            external_id: config.origin.host_str().unwrap_or("relay").into(),
            display_name: None,
            credential_ref: Some(reference),
            settings: Some(pending_settings),
        })
    }
    async fn probe_capabilities(
        &self,
        connection: &ConnectionContext,
    ) -> Result<CapabilitySnapshot, ProviderError> {
        Ok(CapabilitySnapshot {
            connection_id: connection.connection.id,
            capabilities: Capabilities::TOKEN_TOTAL | Capabilities::REQUEST_COUNT,
            observed_at: Utc::now(),
        })
    }
    async fn sync(
        &self,
        context: &ConnectionContext,
        _cursor: Option<SyncCursor>,
    ) -> Result<SyncBatch, ProviderError> {
        let config = settings(&context.settings, &context.connection.connection_type)?;
        let secret = context
            .auth_secret
            .as_ref()
            .ok_or(ProviderError::AuthenticationRequired)?;
        match config.profile.as_str() {
            "new_api" => self.sync_new_api(context, &config, secret).await,
            "openrouter" => self.sync_openrouter(context, &config, secret).await,
            "generic" => Ok(SyncBatch {
                capability_snapshot: Some(self.probe_capabilities(context).await?),
                provider_timestamp: Some(Utc::now()),
                ..Default::default()
            }),
            _ => Err(ProviderError::CapabilityUnavailable),
        }
    }
    async fn disconnect(
        &self,
        connection: &ConnectionContext,
        secrets: &dyn SecretStore,
    ) -> Result<(), ProviderError> {
        if let Some(reference) = &connection.credential_ref {
            secrets.delete(reference).await?;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct TokenUsage {
    total_granted: Value,
    total_used: Value,
    total_available: Value,
    unlimited_quota: Option<bool>,
    expires_at: Option<i64>,
}
#[derive(Deserialize)]
struct NewApiLog {
    request_id: Option<String>,
    upstream_request_id: Option<String>,
    created_at: i64,
    model_name: Option<String>,
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    quota: Option<Value>,
    token_id: Option<i64>,
    token_name: Option<String>,
    #[serde(rename = "type")]
    log_type: Option<i64>,
    is_stream: Option<bool>,
}
#[derive(Deserialize)]
struct OpenRouterActivity {
    date: String,
    model: Option<String>,
    provider_name: Option<String>,
    endpoint_id: Option<String>,
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    requests: Option<u32>,
    usage: Option<Value>,
    byok_usage_inference: Option<Value>,
}
#[derive(Deserialize)]
struct Credits {
    total_credits: Value,
    total_usage: Value,
}

fn new_api_event(
    connection_id: Uuid,
    log: NewApiLog,
    observed_at: DateTime<Utc>,
) -> Result<UsageEvent, ProviderError> {
    let occurred_at = Utc
        .timestamp_opt(log.created_at, 0)
        .single()
        .ok_or(ProviderError::InvalidResponse)?;
    let external_id = log
        .request_id
        .or(log.upstream_request_id)
        .unwrap_or_else(|| {
            fingerprint(&[
                &log.created_at.to_string(),
                log.model_name.as_deref().unwrap_or(""),
                &log.prompt_tokens.unwrap_or_default().to_string(),
                &log.completion_tokens.unwrap_or_default().to_string(),
                &log.token_id.unwrap_or_default().to_string(),
            ])
        });
    let mut dimensions = BTreeMap::new();
    if let Some(id) = log.token_id {
        dimensions.insert("api_key_id".into(), id.to_string());
    }
    if let Some(name) = log.token_name {
        dimensions.insert("api_key_name".into(), name);
    }
    if let Some(stream) = log.is_stream {
        dimensions.insert("streaming".into(), stream.to_string());
    }
    let total_tokens = match (log.prompt_tokens, log.completion_tokens) {
        (Some(a), Some(b)) => Some(a + b),
        _ => None,
    };
    let event = UsageEvent {
        id: Uuid::new_v4(),
        connection_id,
        external_id,
        occurred_at,
        observed_at,
        model: log.model_name,
        input_tokens: log.prompt_tokens,
        cached_input_tokens: None,
        output_tokens: log.completion_tokens,
        reasoning_tokens: None,
        total_tokens,
        request_count: 1,
        actual_charge: None,
        upstream_charge: None,
        estimated_charge: None,
        credit_used: log.quota.as_ref().and_then(decimal_value),
        provenance: Provenance::ProviderReported,
        source_event: "new_api.log".into(),
        dimensions,
    };
    event
        .validate()
        .map_err(|_| ProviderError::InvalidResponse)?;
    Ok(event)
}

fn openrouter_event(
    connection_id: Uuid,
    row: OpenRouterActivity,
    observed_at: DateTime<Utc>,
) -> Result<UsageEvent, ProviderError> {
    let occurred_at = format!("{}T00:00:00Z", row.date)
        .parse()
        .map_err(|_| ProviderError::InvalidResponse)?;
    let external_id = fingerprint(&[
        &row.date,
        row.model.as_deref().unwrap_or(""),
        row.provider_name.as_deref().unwrap_or(""),
        row.endpoint_id.as_deref().unwrap_or(""),
    ]);
    let mut dimensions = BTreeMap::new();
    if let Some(value) = row.provider_name {
        dimensions.insert("upstream_provider".into(), value);
    }
    if let Some(value) = row.endpoint_id {
        dimensions.insert("endpoint_id".into(), value);
    }
    let input = row.prompt_tokens;
    let output = row.completion_tokens;
    let total_tokens = match (input, output) {
        (Some(a), Some(b)) => Some(a + b),
        _ => None,
    };
    Ok(UsageEvent {
        id: Uuid::new_v4(),
        connection_id,
        external_id,
        occurred_at,
        observed_at,
        model: row.model,
        input_tokens: input,
        cached_input_tokens: None,
        output_tokens: output,
        reasoning_tokens: row.reasoning_tokens,
        total_tokens,
        request_count: row.requests.unwrap_or(1).max(1),
        actual_charge: row
            .usage
            .as_ref()
            .and_then(decimal_value)
            .map(|value| MeasuredAmount {
                value,
                unit: MetricUnit::Credit,
            }),
        upstream_charge: row
            .byok_usage_inference
            .as_ref()
            .and_then(decimal_value)
            .map(|value| MeasuredAmount {
                value,
                unit: MetricUnit::Credit,
            }),
        estimated_charge: None,
        credit_used: None,
        provenance: Provenance::ProviderReported,
        source_event: "openrouter.activity".into(),
        dimensions,
    })
}

fn decode_data<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, ProviderError> {
    serde_json::from_value(value.get("data").cloned().unwrap_or(value))
        .map_err(|_| ProviderError::InvalidResponse)
}
fn decode_list<T: for<'de> Deserialize<'de>>(value: Value) -> Result<Vec<T>, ProviderError> {
    let value = value.get("data").cloned().unwrap_or(value);
    let value = value.get("items").cloned().unwrap_or(value);
    serde_json::from_value(value).map_err(|_| ProviderError::InvalidResponse)
}
fn decimal_value(value: &Value) -> Option<Decimal> {
    match value {
        Value::String(v) => v.parse().ok(),
        Value::Number(v) => v.to_string().parse().ok(),
        _ => None,
    }
}
fn fingerprint(fields: &[&str]) -> String {
    let mut hash = Sha256::new();
    for field in fields {
        hash.update(field.as_bytes());
        hash.update([0]);
    }
    hex::encode(hash.finalize())
}
fn events_max_time(events: &[UsageEvent]) -> Option<DateTime<Utc>> {
    events.iter().map(|event| event.occurred_at).max()
}
fn map_status(status: StatusCode) -> Result<(), ProviderError> {
    match status.as_u16() {
        200..=299 => Ok(()),
        401 => Err(ProviderError::AuthenticationRequired),
        403 => Err(ProviderError::PermissionDenied),
        429 => Err(ProviderError::RateLimited { retry_at: None }),
        500..=599 => Err(ProviderError::NetworkUnavailable),
        _ => Err(ProviderError::InvalidResponse),
    }
}
fn network_error(error: reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError::Timeout
    } else {
        ProviderError::NetworkUnavailable
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_secret_and_remote_http_settings() {
        let secret = ConnectionSettings {
            schema_version: 1,
            values: BTreeMap::from([
                (
                    "origin".into(),
                    Value::String("https://relay.example".into()),
                ),
                ("api_key".into(), Value::String("bad".into())),
            ]),
        };
        assert!(settings(&secret, "new_api").is_err());
        let http = ConnectionSettings {
            schema_version: 1,
            values: BTreeMap::from([(
                "origin".into(),
                Value::String("http://relay.example".into()),
            )]),
        };
        assert!(settings(&http, "new_api").is_err());
    }
    #[test]
    fn normalizes_new_api_log() {
        let event = new_api_event(
            Uuid::nil(),
            NewApiLog {
                request_id: Some("r1".into()),
                upstream_request_id: None,
                created_at: 1_700_000_000,
                model_name: Some("gpt".into()),
                prompt_tokens: Some(10),
                completion_tokens: Some(4),
                quota: Some(Value::String("2.5".into())),
                token_id: None,
                token_name: None,
                log_type: Some(2),
                is_stream: Some(true),
            },
            Utc::now(),
        )
        .unwrap();
        assert_eq!(event.total_tokens, Some(14));
        assert_eq!(event.credit_used, Some(Decimal::new(25, 1)));
    }
}
