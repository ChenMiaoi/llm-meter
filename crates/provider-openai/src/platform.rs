use async_trait::async_trait;
use chrono::{DateTime, Utc};
use llm_meter_core::*;
use rust_decimal::Decimal;
use secrecy::ExposeSecret;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use uuid::Uuid;

pub struct AdminAdapter {
    client: reqwest::Client,
    base: String,
}
impl Default for AdminAdapter {
    fn default() -> Self {
        Self::new("https://api.openai.com/v1")
    }
}
impl AdminAdapter {
    pub fn new(base: impl Into<String>) -> Self {
        let base = base.into();
        let parsed = url::Url::parse(&base).expect("valid OpenAI base URL");
        let local = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
        assert!(
            parsed.scheme() == "https" || local,
            "OpenAI endpoints require HTTPS"
        );
        Self {
            client: reqwest::Client::builder()
                .https_only(false)
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("valid client"),
            base,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema_tolerates_unknown_usage_fields() {
        let page:Page<UsageResult>=serde_json::from_value(serde_json::json!({"object":"page","data":[{"start_time":1,"end_time":2,"unknown_bucket":true,"results":[{"input_tokens":10,"input_cached_tokens":4,"output_tokens":3,"num_model_requests":1,"model":"gpt-test","new_field":{"nested":true}}]}],"has_more":false,"next_page":null,"future":"ignored"})).unwrap();
        assert_eq!(page.data[0].results[0].input_cached_tokens, Some(4));
    }
    #[test]
    fn costs_accept_number_or_decimal_string() {
        assert_eq!(
            number_decimal(&serde_json::json!(0.06)),
            Some(Decimal::new(6, 2))
        );
        assert_eq!(
            number_decimal(&serde_json::json!("12.43")),
            Some(Decimal::new(1243, 2))
        );
    }
    #[test]
    #[should_panic(expected = "require HTTPS")]
    fn remote_plain_http_is_rejected() {
        let _ = AdminAdapter::new("http://api.example.com/v1");
    }
}

#[async_trait]
impl ProviderAdapter for AdminAdapter {
    fn manifest(&self) -> ProviderManifest {
        crate::manifest()
    }
    fn supported_auth_schemes(&self) -> Vec<AuthScheme> {
        vec![AuthScheme::AdminApiKey]
    }
    async fn begin_auth(&self, r: BeginAuthRequest) -> Result<AuthChallenge, ProviderError> {
        if r.auth_scheme != AuthScheme::AdminApiKey {
            return Err(ProviderError::CapabilityUnavailable);
        }
        Ok(AuthChallenge::SecretInput {
            challenge_id: String::new(),
            label: "OpenAI Admin API Key".into(),
            placeholder: Some("sk-admin-…".into()),
        })
    }
    async fn complete_auth(
        &self,
        r: CompleteAuthRequest,
        secrets: &dyn SecretStore,
    ) -> Result<ConnectionIdentity, ProviderError> {
        let secret = r.secret.ok_or(ProviderError::AuthenticationRequired)?;
        self.validate(&secret).await?;
        let key = format!("connection_{}", Uuid::new_v4());
        let reference = secrets
            .put("io.github.llmmeter.openai", &key, secret)
            .await?;
        Ok(ConnectionIdentity {
            external_id: "openai-organization".into(),
            display_name: Some("OpenAI Platform".into()),
            credential_ref: Some(reference),
            settings: None,
        })
    }
    async fn probe_capabilities(
        &self,
        c: &ConnectionContext,
    ) -> Result<CapabilitySnapshot, ProviderError> {
        self.validate(secret(c)?).await?;
        Ok(CapabilitySnapshot {
            connection_id: c.connection.id,
            capabilities: Capabilities::TOKEN_INPUT
                | Capabilities::TOKEN_OUTPUT
                | Capabilities::TOKEN_CACHED_INPUT
                | Capabilities::REQUEST_COUNT
                | Capabilities::COST_ACTUAL
                | Capabilities::COST_ESTIMATED
                | Capabilities::PER_MODEL
                | Capabilities::PER_PROJECT
                | Capabilities::PER_API_KEY,
            observed_at: Utc::now(),
        })
    }
    async fn sync(
        &self,
        c: &ConnectionContext,
        cursor: Option<SyncCursor>,
    ) -> Result<SyncBatch, ProviderError> {
        let secret = secret(c)?;
        let now = Utc::now();
        let default_start = (now - chrono::Duration::days(31)).timestamp();
        let cursor_start = cursor
            .and_then(|v| v.0.parse().ok())
            .unwrap_or(default_start);
        let start = cursor_start.min((now - chrono::Duration::days(2)).timestamp());
        let usage = self.fetch_usage(secret, start).await?;
        let costs = self.fetch_costs(secret, start).await?;
        let mut metrics = Vec::new();
        for b in usage {
            let ps = DateTime::from_timestamp(b.start_time, 0);
            let pe = DateTime::from_timestamp(b.end_time, 0);
            for r in b.results {
                let dims = dimensions(&r);
                for (key, value) in [
                    (MetricKey::TOKEN_INPUT, r.input_tokens),
                    (MetricKey::TOKEN_CACHED_INPUT, r.input_cached_tokens),
                    (MetricKey::TOKEN_OUTPUT, r.output_tokens),
                    (MetricKey::REQUEST_COUNT, r.num_model_requests),
                ] {
                    if let Some(v) = value {
                        metrics.push(sample(
                            c.connection.id,
                            key,
                            Decimal::from(v),
                            if key == MetricKey::REQUEST_COUNT {
                                MetricUnit::Request
                            } else {
                                MetricUnit::Token
                            },
                            SamplePeriod {
                                start: ps,
                                end: pe,
                                observed: now,
                            },
                            dims.clone(),
                            "organization.usage.completions",
                        ));
                    }
                }
                let is_standard_tier = r
                    .service_tier
                    .as_deref()
                    .is_none_or(|tier| matches!(tier, "default" | "standard"));
                if is_standard_tier
                    && let Some(model) = r.model.as_deref()
                    && let Some(cost) = crate::pricing::estimate_text_tokens(
                        model,
                        r.input_tokens.unwrap_or_default(),
                        r.input_cached_tokens.unwrap_or_default(),
                        r.output_tokens.unwrap_or_default(),
                    )
                {
                    let mut cost_dims = dims;
                    cost_dims.insert("pricing_tier".into(), "standard".into());
                    cost_dims.insert("pricing_as_of".into(), crate::pricing::PRICE_AS_OF.into());
                    let mut estimated = sample(
                        c.connection.id,
                        MetricKey::COST_ESTIMATED,
                        cost,
                        MetricUnit::Currency("USD".into()),
                        SamplePeriod {
                            start: ps,
                            end: pe,
                            observed: now,
                        },
                        cost_dims,
                        "openai.standard_text_token_pricing",
                    );
                    estimated.provenance = Provenance::Estimated;
                    metrics.push(estimated);
                }
            }
        }
        for b in costs {
            let ps = DateTime::from_timestamp(b.start_time, 0);
            let pe = DateTime::from_timestamp(b.end_time, 0);
            for r in b.results {
                if let Some(amount) = r.amount.value.as_ref().and_then(number_decimal) {
                    let mut dims = BTreeMap::new();
                    if let Some(v) = r.project_id {
                        dims.insert("project_id".into(), v);
                    }
                    if let Some(v) = r.line_item {
                        dims.insert("line_item".into(), v);
                    }
                    metrics.push(sample(
                        c.connection.id,
                        MetricKey::COST_ACTUAL,
                        amount,
                        MetricUnit::Currency(r.amount.currency.to_uppercase()),
                        SamplePeriod {
                            start: ps,
                            end: pe,
                            observed: now,
                        },
                        dims,
                        "organization.costs.amount",
                    ));
                }
            }
        }
        Ok(SyncBatch {
            capability_snapshot: Some(self.probe_capabilities(c).await?),
            metric_samples: metrics,
            next_cursor: Some(SyncCursor(now.timestamp().to_string())),
            provider_timestamp: Some(now),
            ..Default::default()
        })
    }
    async fn disconnect(
        &self,
        c: &ConnectionContext,
        secrets: &dyn SecretStore,
    ) -> Result<(), ProviderError> {
        if let Some(r) = &c.credential_ref {
            secrets.delete(r).await?
        }
        Ok(())
    }
}
impl AdminAdapter {
    async fn validate(&self, key: &secrecy::SecretString) -> Result<(), ProviderError> {
        let response = self
            .client
            .get(format!("{}/organization/projects", self.base))
            .query(&[("limit", "1")])
            .bearer_auth(key.expose_secret())
            .send()
            .await
            .map_err(net)?;
        status(response).await.map(|_| ())
    }
    async fn fetch_usage(
        &self,
        key: &secrecy::SecretString,
        start: i64,
    ) -> Result<Vec<Bucket<UsageResult>>, ProviderError> {
        let mut page = None;
        let mut all = Vec::new();
        for _ in 0..100 {
            let mut req = self
                .client
                .get(format!("{}/organization/usage/completions", self.base))
                .query(&[
                    ("start_time", start.to_string()),
                    ("bucket_width", "1d".into()),
                    ("limit", "31".into()),
                    ("group_by", "model".into()),
                    ("group_by", "project_id".into()),
                    ("group_by", "api_key_id".into()),
                ])
                .bearer_auth(key.expose_secret());
            if let Some(p) = &page {
                req = req.query(&[("page", p)]);
            }
            let body: Page<UsageResult> = status(req.send().await.map_err(net)?)
                .await?
                .json()
                .await
                .map_err(|_| ProviderError::InvalidResponse)?;
            all.extend(body.data);
            if !body.has_more {
                break;
            }
            page = body.next_page;
        }
        Ok(all)
    }
    async fn fetch_costs(
        &self,
        key: &secrecy::SecretString,
        start: i64,
    ) -> Result<Vec<Bucket<CostResult>>, ProviderError> {
        let mut page = None;
        let mut all = Vec::new();
        for _ in 0..100 {
            let mut req = self
                .client
                .get(format!("{}/organization/costs", self.base))
                .query(&[
                    ("start_time", start.to_string()),
                    ("bucket_width", "1d".into()),
                    ("limit", "180".into()),
                    ("group_by", "project_id".into()),
                    ("group_by", "line_item".into()),
                ])
                .bearer_auth(key.expose_secret());
            if let Some(p) = &page {
                req = req.query(&[("page", p)]);
            }
            let body: Page<CostResult> = status(req.send().await.map_err(net)?)
                .await?
                .json()
                .await
                .map_err(|_| ProviderError::InvalidResponse)?;
            all.extend(body.data);
            if !body.has_more {
                break;
            }
            page = body.next_page;
        }
        Ok(all)
    }
}
fn secret(c: &ConnectionContext) -> Result<&secrecy::SecretString, ProviderError> {
    c.auth_secret
        .as_ref()
        .ok_or(ProviderError::AuthenticationRequired)
}
async fn status(r: reqwest::Response) -> Result<reqwest::Response, ProviderError> {
    match r.status().as_u16() {
        200..=299 => Ok(r),
        401 => Err(ProviderError::AuthenticationRequired),
        403 => Err(ProviderError::PermissionDenied),
        429 => Err(ProviderError::RateLimited { retry_at: None }),
        500..=599 => Err(ProviderError::NetworkUnavailable),
        _ => Err(ProviderError::InvalidResponse),
    }
}
fn net(e: reqwest::Error) -> ProviderError {
    if e.is_timeout() {
        ProviderError::Timeout
    } else {
        ProviderError::NetworkUnavailable
    }
}
#[derive(Deserialize)]
struct Page<T> {
    data: Vec<Bucket<T>>,
    #[serde(default)]
    has_more: bool,
    next_page: Option<String>,
}
#[derive(Deserialize)]
struct Bucket<T> {
    start_time: i64,
    end_time: i64,
    results: Vec<T>,
}
#[derive(Deserialize)]
struct UsageResult {
    input_tokens: Option<i64>,
    input_cached_tokens: Option<i64>,
    output_tokens: Option<i64>,
    num_model_requests: Option<i64>,
    model: Option<String>,
    project_id: Option<String>,
    api_key_id: Option<String>,
    service_tier: Option<String>,
}
#[derive(Deserialize)]
struct CostResult {
    amount: Amount,
    line_item: Option<String>,
    project_id: Option<String>,
}
#[derive(Deserialize)]
struct Amount {
    value: Option<Value>,
    currency: String,
}
fn dimensions(r: &UsageResult) -> BTreeMap<String, String> {
    let mut d = BTreeMap::new();
    for (k, v) in [
        ("model", &r.model),
        ("project_id", &r.project_id),
        ("api_key_id", &r.api_key_id),
        ("service_tier", &r.service_tier),
    ] {
        if let Some(v) = v {
            d.insert(k.into(), v.clone());
        }
    }
    d
}
fn number_decimal(v: &Value) -> Option<Decimal> {
    match v {
        Value::Number(n) => n.to_string().parse().ok(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

struct SamplePeriod {
    start: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
    observed: DateTime<Utc>,
}

fn sample(
    id: Uuid,
    key: &str,
    value: Decimal,
    unit: MetricUnit,
    period: SamplePeriod,
    dimensions: BTreeMap<String, String>,
    source: &str,
) -> MetricSample {
    let scope = if dimensions.contains_key("api_key_id") {
        MetricScope::ApiKey
    } else if dimensions.contains_key("project_id") {
        MetricScope::Project
    } else if dimensions.contains_key("model") {
        MetricScope::Model
    } else {
        MetricScope::Organization
    };
    let mut m = MetricSample {
        id: Uuid::new_v4(),
        connection_id: id,
        metric_key: MetricKey(key.into()),
        value,
        unit,
        scope,
        period_start: period.start,
        period_end: period.end,
        observed_at: period.observed,
        provenance: Provenance::ProviderReported,
        dimensions,
        source_metric: source.into(),
        dedup_key: String::new(),
    };
    m.dedup_key = m.compute_dedup_key();
    m
}
