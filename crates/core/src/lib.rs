//! Provider-neutral domain contracts. This crate deliberately has no database,
//! HTTP, UI, or desktop dependencies.

use std::collections::BTreeMap;
use std::fmt;

use async_trait::async_trait;
use bitflags::bitflags;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const IPC_VERSION: u32 = 2;
pub const SCHEMA_VERSION: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {
    Connecting,
    Ready,
    Syncing,
    Stale,
    AuthRequired,
    RateLimited,
    Offline,
    ProviderError,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    pub id: Uuid,
    pub provider_id: String,
    pub connection_type: String,
    pub display_name: String,
    pub account_external_id: Option<String>,
    pub status: ConnectionStatus,
    /// Opaque database reference only; never a secret value.
    pub credential_ref_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_error_code: Option<String>,
    pub disabled_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountRecord {
    pub id: Uuid,
    pub connection_id: Uuid,
    pub external_id: String,
    pub display_name: Option<String>,
    pub account_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductRecord {
    pub id: Uuid,
    pub connection_id: Uuid,
    pub product_key: String,
    pub display_name: Option<String>,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Capabilities: u64 {
        const ACCOUNT_INFO = 1 << 0;
        const PLAN_INFO = 1 << 1;
        const QUOTA_WINDOWS = 1 << 2;
        const QUOTA_EVENTS = 1 << 3;
        const TOKEN_TOTAL = 1 << 4;
        const TOKEN_DAILY = 1 << 5;
        const TOKEN_INPUT = 1 << 6;
        const TOKEN_OUTPUT = 1 << 7;
        const TOKEN_CACHED_INPUT = 1 << 8;
        const TOKEN_REASONING = 1 << 9;
        const REQUEST_COUNT = 1 << 10;
        const COST_ACTUAL = 1 << 11;
        const COST_ESTIMATED = 1 << 12;
        const CREDITS_BALANCE = 1 << 13;
        const PER_MODEL = 1 << 14;
        const PER_PROJECT = 1 << 15;
        const PER_API_KEY = 1 << 16;
        const PER_THREAD = 1 << 17;
        const EVENT_STREAM = 1 << 18;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilitySnapshot {
    pub connection_id: Uuid,
    pub capabilities: Capabilities,
    pub observed_at: DateTime<Utc>,
}

impl Capabilities {
    pub fn names(self) -> Vec<&'static str> {
        const ALL: &[(Capabilities, &str)] = &[
            (Capabilities::ACCOUNT_INFO, "account_info"),
            (Capabilities::PLAN_INFO, "plan_info"),
            (Capabilities::QUOTA_WINDOWS, "quota_windows"),
            (Capabilities::QUOTA_EVENTS, "quota_events"),
            (Capabilities::TOKEN_TOTAL, "token_total"),
            (Capabilities::TOKEN_DAILY, "token_daily"),
            (Capabilities::TOKEN_INPUT, "token_input"),
            (Capabilities::TOKEN_OUTPUT, "token_output"),
            (Capabilities::TOKEN_CACHED_INPUT, "token_cached_input"),
            (Capabilities::TOKEN_REASONING, "token_reasoning"),
            (Capabilities::REQUEST_COUNT, "request_count"),
            (Capabilities::COST_ACTUAL, "cost_actual"),
            (Capabilities::COST_ESTIMATED, "cost_estimated"),
            (Capabilities::CREDITS_BALANCE, "credits_balance"),
            (Capabilities::PER_MODEL, "per_model"),
            (Capabilities::PER_PROJECT, "per_project"),
            (Capabilities::PER_API_KEY, "per_api_key"),
            (Capabilities::PER_THREAD, "per_thread"),
            (Capabilities::EVENT_STREAM, "event_stream"),
        ];
        ALL.iter()
            .filter_map(|(f, n)| self.contains(*f).then_some(*n))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MetricKey(pub String);

impl MetricKey {
    pub const TOKEN_TOTAL: &'static str = "token.total";
    pub const TOKEN_INPUT: &'static str = "token.input";
    pub const TOKEN_CACHED_INPUT: &'static str = "token.cached_input";
    pub const TOKEN_OUTPUT: &'static str = "token.output";
    pub const TOKEN_REASONING_OUTPUT: &'static str = "token.reasoning_output";
    pub const REQUEST_COUNT: &'static str = "request.count";
    pub const COST_ACTUAL: &'static str = "cost.actual";
    pub const COST_ESTIMATED: &'static str = "cost.estimated";
    pub const CREDIT_BALANCE: &'static str = "credit.balance";
    pub const COST_UPSTREAM: &'static str = "cost.upstream";
    pub const CREDIT_USED: &'static str = "credit.used";
    pub const BUDGET_CONFIGURED: &'static str = "budget.configured";
    pub const BUDGET_REMAINING: &'static str = "budget.remaining";

    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.split('.').all(|part| {
                !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase() || c == '_')
            });
        valid
            .then_some(Self(value))
            .ok_or(DomainError::InvalidMetricKey)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "code", rename_all = "snake_case")]
pub enum MetricUnit {
    Token,
    Request,
    Ratio,
    Percent,
    Credit,
    Currency(String),
    Second,
    Count,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricScope {
    Account,
    Organization,
    Workspace,
    Project,
    ApiKey,
    Model,
    Subscription,
    Product,
    Thread,
    Device,
    LocalProxy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    ProviderReported,
    LocallyObserved,
    Derived,
    Estimated,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSample {
    pub id: Uuid,
    pub connection_id: Uuid,
    pub metric_key: MetricKey,
    #[serde(with = "rust_decimal::serde::str")]
    pub value: Decimal,
    pub unit: MetricUnit,
    pub scope: MetricScope,
    pub period_start: Option<DateTime<Utc>>,
    pub period_end: Option<DateTime<Utc>>,
    pub observed_at: DateTime<Utc>,
    pub provenance: Provenance,
    #[serde(default)]
    pub dimensions: BTreeMap<String, String>,
    pub source_metric: String,
    pub dedup_key: String,
}

impl MetricSample {
    pub fn compute_dedup_key(&self) -> String {
        let mut h = Sha256::new();
        for field in [
            self.connection_id.to_string(),
            self.metric_key.0.clone(),
            format!("{:?}", self.scope),
            self.period_start
                .map(|v| v.to_rfc3339())
                .unwrap_or_default(),
            self.period_end.map(|v| v.to_rfc3339()).unwrap_or_default(),
            serde_json::to_string(&self.dimensions).expect("BTreeMap serialization cannot fail"),
            self.source_metric.clone(),
        ] {
            h.update(field.as_bytes());
            h.update([0]);
        }
        hex::encode(h.finalize())
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        if let (Some(start), Some(end)) = (self.period_start, self.period_end) {
            if start >= end {
                return Err(DomainError::InvalidPeriod);
            }
        }
        if self.unit == MetricUnit::Ratio && !(Decimal::ZERO..=Decimal::ONE).contains(&self.value) {
            return Err(DomainError::InvalidRatio);
        }
        if self.dedup_key != self.compute_dedup_key() {
            return Err(DomainError::InvalidDedupKey);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    Rolling,
    Fixed,
    Daily,
    Weekly,
    Monthly,
    BillingCycle,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaWindow {
    pub id: Uuid,
    pub connection_id: Uuid,
    pub provider_limit_id: String,
    pub display_name: Option<String>,
    pub window_kind: WindowKind,
    pub window_start: Option<DateTime<Utc>>,
    pub window_end: Option<DateTime<Utc>>,
    pub resets_at: Option<DateTime<Utc>>,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub used_ratio: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub remaining_ratio: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub used_value: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub limit_value: Option<Decimal>,
    pub unit: Option<MetricUnit>,
    pub provenance: Provenance,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitResetCredit {
    pub id: String,
    pub connection_id: Uuid,
    pub reset_type: String,
    pub status: String,
    pub granted_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub title: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitResetCredits {
    pub connection_id: Uuid,
    pub available_count: u64,
    /// `None` means the provider only returned the total count. An empty vector
    /// means details were fetched and there are no available credits.
    pub credits: Option<Vec<RateLimitResetCredit>>,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetPeriod {
    Daily,
    Weekly,
    Monthly,
    BillingCycle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Budget {
    pub id: Uuid,
    pub connection_id: Uuid,
    #[serde(with = "rust_decimal::serde::str")]
    pub amount: Decimal,
    pub currency: String,
    pub period: BudgetPeriod,
    #[serde(with = "rust_decimal::serde::str")]
    pub warning_ratio: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub critical_ratio: Decimal,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertKind {
    QuotaRemaining,
    QuotaResetSoon,
    TokenDaily,
    CostBudgetRatio,
    CacheHitRatio,
    AuthenticationRequired,
    DataStale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertState {
    Normal,
    Triggered,
    Acknowledged,
    Suppressed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertRule {
    pub id: Uuid,
    pub connection_id: Uuid,
    pub kind: AlertKind,
    #[serde(with = "rust_decimal::serde::str")]
    pub threshold: Decimal,
    pub state: AlertState,
    pub last_triggered_at: Option<DateTime<Utc>>,
    pub suppressed_until: Option<DateTime<Utc>>,
}

impl QuotaWindow {
    pub fn normalize(mut self) -> Result<Self, DomainError> {
        for v in [self.used_ratio, self.remaining_ratio]
            .into_iter()
            .flatten()
        {
            if !(Decimal::ZERO..=Decimal::ONE).contains(&v) {
                return Err(DomainError::InvalidRatio);
            }
        }
        if self.remaining_ratio.is_none() {
            self.remaining_ratio = self.used_ratio.map(|v| Decimal::ONE - v);
        }
        if self.used_ratio.is_none() {
            self.used_ratio = self.remaining_ratio.map(|v| Decimal::ONE - v);
        }
        Ok(self)
    }
}

pub fn cache_hit_ratio(input: &MetricSample, cached: &MetricSample) -> Option<Decimal> {
    let compatible = input.connection_id == cached.connection_id
        && input.scope == cached.scope
        && input.period_start == cached.period_start
        && input.period_end == cached.period_end
        && input.dimensions == cached.dimensions
        && input.metric_key.0 == MetricKey::TOKEN_INPUT
        && cached.metric_key.0 == MetricKey::TOKEN_CACHED_INPUT
        && input.unit == MetricUnit::Token
        && cached.unit == MetricUnit::Token
        && input.value > Decimal::ZERO;
    compatible.then(|| cached.value / input.value)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionTypeManifest {
    pub id: String,
    pub display_name: String,
    pub auth_schemes: Vec<AuthScheme>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderManifest {
    pub provider_id: String,
    pub display_name: String,
    pub adapter_version: String,
    pub connection_types: Vec<ConnectionTypeManifest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthScheme {
    #[serde(rename = "oauth_browser", alias = "o_auth_browser")]
    OAuthBrowser,
    #[serde(rename = "oauth_device_code", alias = "o_auth_device_code")]
    OAuthDeviceCode,
    ApiKey,
    AdminApiKey,
    ServiceAccount,
    PersonalAccessToken,
    LocalSessionBridge,
    LocalProxy,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ConnectionSettings {
    pub schema_version: u32,
    #[serde(default)]
    pub values: BTreeMap<String, serde_json::Value>,
}

impl ConnectionSettings {
    pub fn validate_public(&self) -> Result<(), DomainError> {
        const SECRET_TERMS: &[&str] = &["api_key", "authorization", "password", "secret", "token"];
        if self.values.keys().any(|key| {
            let normalized = key.to_ascii_lowercase();
            SECRET_TERMS.iter().any(|term| normalized.contains(term))
        }) {
            return Err(DomainError::SecretInSettings);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeginAuthRequest {
    pub connection_type: String,
    pub auth_scheme: AuthScheme,
    pub display_name: String,
    #[serde(default)]
    pub settings: ConnectionSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthChallenge {
    Browser {
        auth_url: String,
        state: String,
    },
    DeviceCode {
        verification_url: String,
        user_code: String,
        state: String,
        expires_at: DateTime<Utc>,
        interval_seconds: u64,
    },
    SecretInput {
        challenge_id: String,
        label: String,
        placeholder: Option<String>,
    },
    Complete,
}

/// Secret material is deliberately non-serializable.
#[derive(Debug)]
pub struct CompleteAuthRequest {
    pub challenge_state: Option<String>,
    pub secret: Option<SecretString>,
    pub connection_type: Option<String>,
    pub settings: Option<ConnectionSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionIdentity {
    pub external_id: String,
    pub display_name: Option<String>,
    pub credential_ref: Option<CredentialRef>,
    pub settings: Option<ConnectionSettings>,
}

#[derive(Debug, Clone)]
pub struct ConnectionContext {
    pub connection: Connection,
    pub credential_ref: Option<CredentialRef>,
    /// Ephemeral material loaded by Provider Runtime. `SecretString` redacts its
    /// Debug representation and cannot be serialized.
    pub auth_secret: Option<SecretString>,
    pub settings: ConnectionSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeasuredAmount {
    #[serde(with = "rust_decimal::serde::str")]
    pub value: Decimal,
    pub unit: MetricUnit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEvent {
    pub id: Uuid,
    pub connection_id: Uuid,
    pub external_id: String,
    pub occurred_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub model: Option<String>,
    pub input_tokens: Option<i64>,
    pub cached_input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub request_count: u32,
    pub actual_charge: Option<MeasuredAmount>,
    pub upstream_charge: Option<MeasuredAmount>,
    pub estimated_charge: Option<MeasuredAmount>,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub credit_used: Option<Decimal>,
    pub provenance: Provenance,
    pub source_event: String,
    #[serde(default)]
    pub dimensions: BTreeMap<String, String>,
}

impl UsageEvent {
    pub fn validate(&self) -> Result<(), DomainError> {
        let tokens = [
            self.input_tokens,
            self.cached_input_tokens,
            self.output_tokens,
            self.reasoning_tokens,
            self.total_tokens,
        ];
        if self.external_id.trim().is_empty()
            || self.request_count == 0
            || tokens.into_iter().flatten().any(|value| value < 0)
            || self.credit_used.is_some_and(|value| value < Decimal::ZERO)
            || [
                &self.actual_charge,
                &self.upstream_charge,
                &self.estimated_charge,
            ]
            .into_iter()
            .flatten()
            .any(|amount| amount.value < Decimal::ZERO)
        {
            return Err(DomainError::InvalidUsageEvent);
        }
        MetricKey::new(self.source_event.clone()).map_err(|_| DomainError::InvalidUsageEvent)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderEvent {
    pub id: Uuid,
    pub connection_id: Uuid,
    pub event_type: String,
    pub observed_at: DateTime<Utc>,
    #[serde(default)]
    pub summary: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncCursor(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SyncBatch {
    pub account_updates: Vec<AccountRecord>,
    pub product_updates: Vec<ProductRecord>,
    pub capability_snapshot: Option<CapabilitySnapshot>,
    pub metric_samples: Vec<MetricSample>,
    pub quota_windows: Vec<QuotaWindow>,
    pub rate_limit_reset_credits: Option<RateLimitResetCredits>,
    pub usage_events: Vec<UsageEvent>,
    pub provider_events: Vec<ProviderEvent>,
    pub next_cursor: Option<SyncCursor>,
    pub provider_timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialRef {
    pub id: Uuid,
    pub backend: String,
    pub service_name: String,
    pub secret_key: String,
    pub created_at: DateTime<Utc>,
}

#[async_trait]
pub trait SecretStore: Send + Sync {
    async fn put(
        &self,
        service: &str,
        key: &str,
        secret: SecretString,
    ) -> Result<CredentialRef, ProviderError>;
    async fn get(&self, reference: &CredentialRef) -> Result<SecretString, ProviderError>;
    async fn delete(&self, reference: &CredentialRef) -> Result<(), ProviderError>;
    async fn available(&self) -> bool;
}

#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn manifest(&self) -> ProviderManifest;
    fn supported_auth_schemes(&self) -> Vec<AuthScheme>;
    async fn begin_auth(&self, request: BeginAuthRequest) -> Result<AuthChallenge, ProviderError>;
    async fn complete_auth(
        &self,
        request: CompleteAuthRequest,
        secrets: &dyn SecretStore,
    ) -> Result<ConnectionIdentity, ProviderError>;
    async fn probe_capabilities(
        &self,
        connection: &ConnectionContext,
    ) -> Result<CapabilitySnapshot, ProviderError>;
    async fn sync(
        &self,
        connection: &ConnectionContext,
        cursor: Option<SyncCursor>,
    ) -> Result<SyncBatch, ProviderError>;
    async fn disconnect(
        &self,
        connection: &ConnectionContext,
        secrets: &dyn SecretStore,
    ) -> Result<(), ProviderError>;

    /// Optional hook called before each sync to refresh expired credentials.
    /// Returns a new secret string when the provider refreshed the credential;
    /// the runtime updates the secret store and the connection context before
    /// invoking `sync`.
    async fn refresh_credentials(
        &self,
        _connection: &ConnectionContext,
        _secrets: &dyn SecretStore,
    ) -> Result<Option<SecretString>, ProviderError> {
        Ok(None)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("authentication required")]
    AuthenticationRequired,
    #[error("permission denied")]
    PermissionDenied,
    #[error("rate limited")]
    RateLimited { retry_at: Option<DateTime<Utc>> },
    #[error("network unavailable")]
    NetworkUnavailable,
    #[error("request timed out")]
    Timeout,
    #[error("invalid provider response")]
    InvalidResponse,
    #[error("unsupported provider version")]
    UnsupportedVersion,
    #[error("capability unavailable")]
    CapabilityUnavailable,
    #[error("secret store unavailable")]
    SecretStoreUnavailable,
    #[error("internal provider error: {0}")]
    Internal(String),
}

impl ProviderError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::AuthenticationRequired => "authentication_required",
            Self::PermissionDenied => "permission_denied",
            Self::RateLimited { .. } => "rate_limited",
            Self::NetworkUnavailable => "network_unavailable",
            Self::Timeout => "timeout",
            Self::InvalidResponse => "invalid_response",
            Self::UnsupportedVersion => "unsupported_version",
            Self::CapabilityUnavailable => "capability_unavailable",
            Self::SecretStoreUnavailable => "secret_store_unavailable",
            Self::Internal(_) => "internal",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("invalid metric key")]
    InvalidMetricKey,
    #[error("period start must precede period end")]
    InvalidPeriod,
    #[error("ratio must be between zero and one")]
    InvalidRatio,
    #[error("dedup key is not canonical")]
    InvalidDedupKey,
    #[error("connection settings contain secret material")]
    SecretInSettings,
    #[error("invalid usage event")]
    InvalidUsageEvent,
}

impl fmt::Display for MetricKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

pub mod pricing;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rust_decimal::Decimal;

    fn metric(key: &str, value: i64) -> MetricSample {
        let mut m = MetricSample {
            id: Uuid::new_v4(),
            connection_id: Uuid::nil(),
            metric_key: MetricKey(key.into()),
            value: Decimal::new(value, 0),
            unit: MetricUnit::Token,
            scope: MetricScope::Organization,
            period_start: Some(Utc.with_ymd_and_hms(2026, 7, 13, 0, 0, 0).unwrap()),
            period_end: Some(Utc.with_ymd_and_hms(2026, 7, 14, 0, 0, 0).unwrap()),
            observed_at: Utc::now(),
            provenance: Provenance::ProviderReported,
            dimensions: BTreeMap::new(),
            source_metric: key.into(),
            dedup_key: String::new(),
        };
        m.dedup_key = m.compute_dedup_key();
        m
    }

    #[test]
    fn stable_dedup_key() {
        let a = metric(MetricKey::TOKEN_INPUT, 10);
        let mut b = a.clone();
        b.value = Decimal::new(99, 0);
        assert_eq!(a.compute_dedup_key(), b.compute_dedup_key());
    }
    #[test]
    fn compatible_cache_ratio() {
        assert_eq!(
            cache_hit_ratio(
                &metric(MetricKey::TOKEN_INPUT, 100),
                &metric(MetricKey::TOKEN_CACHED_INPUT, 40)
            ),
            Some(Decimal::new(4, 1))
        );
    }
    #[test]
    fn incompatible_cache_ratio_is_missing() {
        let a = metric(MetricKey::TOKEN_INPUT, 100);
        let mut b = metric(MetricKey::TOKEN_CACHED_INPUT, 40);
        b.scope = MetricScope::Project;
        assert_eq!(cache_hit_ratio(&a, &b), None);
    }
    #[test]
    fn missing_quota_is_not_zero() {
        let q = QuotaWindow {
            id: Uuid::new_v4(),
            connection_id: Uuid::nil(),
            provider_limit_id: "x".into(),
            display_name: None,
            window_kind: WindowKind::Unknown,
            window_start: None,
            window_end: None,
            resets_at: None,
            used_ratio: None,
            remaining_ratio: None,
            used_value: None,
            limit_value: None,
            unit: None,
            provenance: Provenance::ProviderReported,
            observed_at: Utc::now(),
        }
        .normalize()
        .unwrap();
        assert_eq!(q.remaining_ratio, None);
    }

    #[test]
    fn oauth_wire_names_match_public_ipc_contract() {
        assert_eq!(
            serde_json::to_string(&AuthScheme::OAuthBrowser).unwrap(),
            "\"oauth_browser\""
        );
        assert_eq!(
            serde_json::from_str::<AuthScheme>("\"oauth_device_code\"").unwrap(),
            AuthScheme::OAuthDeviceCode
        );
    }
    #[test]
    fn usage_event_rejects_negative_values() {
        let event = UsageEvent {
            id: Uuid::new_v4(),
            connection_id: Uuid::nil(),
            external_id: "request-1".into(),
            occurred_at: Utc::now(),
            observed_at: Utc::now(),
            model: None,
            input_tokens: Some(-1),
            cached_input_tokens: None,
            output_tokens: None,
            reasoning_tokens: None,
            total_tokens: None,
            request_count: 1,
            actual_charge: None,
            upstream_charge: None,
            estimated_charge: None,
            credit_used: None,
            provenance: Provenance::ProviderReported,
            source_event: "relay.usage".into(),
            dimensions: BTreeMap::new(),
        };
        assert!(matches!(
            event.validate(),
            Err(DomainError::InvalidUsageEvent)
        ));
    }

    #[test]
    fn settings_reject_secret_keys() {
        let settings = ConnectionSettings {
            schema_version: 1,
            values: BTreeMap::from([("api_key".into(), serde_json::json!("secret"))]),
        };
        assert!(matches!(
            settings.validate_public(),
            Err(DomainError::SecretInSettings)
        ));
    }
}
