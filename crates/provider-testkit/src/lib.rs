use async_trait::async_trait;
use chrono::Utc;
use llm_meter_core::*;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ContractViolation {
    #[error("provider error: {0}")]
    Provider(#[from] ProviderError),
    #[error("manifest does not describe an adapter auth scheme")]
    ManifestAuthMismatch,
    #[error("adapter returned data for another connection")]
    CrossConnectionData,
    #[error("adapter returned a non-canonical or invalid metric: {0}")]
    InvalidMetric(String),
    #[error("adapter returned duplicate dedup keys in a batch")]
    DuplicateMetric,
    #[error("adapter returned an invalid quota window: {0}")]
    InvalidQuota(String),
    #[error("capability snapshot contradicts returned data")]
    CapabilityMismatch,
}

/// Shared provider contract checks. Fixture-based adapters can run this without
/// network access; live adapters should use a dedicated test account.
pub async fn verify_contract(
    adapter: &dyn ProviderAdapter,
    context: &ConnectionContext,
) -> Result<(), ContractViolation> {
    let manifest = adapter.manifest();
    let schemes = adapter.supported_auth_schemes();
    if !schemes.iter().all(|scheme| {
        manifest
            .connection_types
            .iter()
            .any(|kind| kind.auth_schemes.contains(scheme))
    }) {
        return Err(ContractViolation::ManifestAuthMismatch);
    }
    let capabilities = adapter.probe_capabilities(context).await?;
    let batch = adapter.sync(context, None).await?;
    let mut dedup = std::collections::HashSet::new();
    for metric in &batch.metric_samples {
        if metric.connection_id != context.connection.id {
            return Err(ContractViolation::CrossConnectionData);
        }
        metric
            .validate()
            .map_err(|error| ContractViolation::InvalidMetric(error.to_string()))?;
        if !dedup.insert(&metric.dedup_key) {
            return Err(ContractViolation::DuplicateMetric);
        }
    }
    if batch
        .quota_windows
        .iter()
        .any(|quota| quota.connection_id != context.connection.id)
    {
        return Err(ContractViolation::CrossConnectionData);
    }
    for quota in &batch.quota_windows {
        quota
            .clone()
            .normalize()
            .map_err(|error| ContractViolation::InvalidQuota(error.to_string()))?;
    }
    if !batch.quota_windows.is_empty()
        && !capabilities
            .capabilities
            .contains(Capabilities::QUOTA_WINDOWS)
    {
        return Err(ContractViolation::CapabilityMismatch);
    }
    Ok(())
}

#[derive(Debug, Default)]
pub struct MockProvider;
#[async_trait]
impl ProviderAdapter for MockProvider {
    fn manifest(&self) -> ProviderManifest {
        ProviderManifest {
            provider_id: "mock".into(),
            display_name: "Mock Provider".into(),
            adapter_version: "0.1.0".into(),
            connection_types: vec![ConnectionTypeManifest {
                id: "fixture".into(),
                display_name: "Fixture".into(),
                auth_schemes: vec![AuthScheme::Manual],
            }],
        }
    }
    fn supported_auth_schemes(&self) -> Vec<AuthScheme> {
        vec![AuthScheme::Manual]
    }
    async fn begin_auth(&self, _: BeginAuthRequest) -> Result<AuthChallenge, ProviderError> {
        Ok(AuthChallenge::Complete)
    }
    async fn complete_auth(
        &self,
        _: CompleteAuthRequest,
        _: &dyn SecretStore,
    ) -> Result<ConnectionIdentity, ProviderError> {
        Ok(ConnectionIdentity {
            external_id: "mock-account".into(),
            display_name: Some("Mock Account".into()),
            credential_ref: None,
        })
    }
    async fn probe_capabilities(
        &self,
        c: &ConnectionContext,
    ) -> Result<CapabilitySnapshot, ProviderError> {
        Ok(CapabilitySnapshot {
            connection_id: c.connection.id,
            capabilities: Capabilities::ACCOUNT_INFO
                | Capabilities::QUOTA_WINDOWS
                | Capabilities::TOKEN_TOTAL,
            observed_at: Utc::now(),
        })
    }
    async fn sync(
        &self,
        c: &ConnectionContext,
        _: Option<SyncCursor>,
    ) -> Result<SyncBatch, ProviderError> {
        Ok(SyncBatch {
            account_updates: vec![AccountRecord {
                id: Uuid::new_v4(),
                connection_id: c.connection.id,
                external_id: "mock-account".into(),
                display_name: Some("Mock Account".into()),
                account_type: Some("fixture".into()),
            }],
            capability_snapshot: Some(self.probe_capabilities(c).await?),
            next_cursor: Some(SyncCursor("1".into())),
            ..Default::default()
        })
    }
    async fn disconnect(
        &self,
        _: &ConnectionContext,
        _: &dyn SecretStore,
    ) -> Result<(), ProviderError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_meter_secret_store::MemorySecretStore;

    #[tokio::test]
    async fn mock_satisfies_provider_contract() {
        let now = Utc::now();
        let context = ConnectionContext {
            connection: Connection {
                id: Uuid::new_v4(),
                provider_id: "mock".into(),
                connection_type: "fixture".into(),
                display_name: "Mock".into(),
                account_external_id: None,
                status: ConnectionStatus::Ready,
                credential_ref_id: None,
                created_at: now,
                updated_at: now,
                last_success_at: None,
                last_error_code: None,
                disabled_at: None,
            },
            credential_ref: None,
            auth_secret: None,
        };
        verify_contract(&MockProvider, &context).await.unwrap();
        let _ = MemorySecretStore::default();
    }
}
