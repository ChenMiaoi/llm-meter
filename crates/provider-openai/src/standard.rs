use async_trait::async_trait;
use chrono::Utc;
use llm_meter_core::*;
use secrecy::ExposeSecret;
use uuid::Uuid;

/// Standard API keys can be connection-checked in v0.1. They intentionally do
/// not claim organization history; locally observed metrics belong to the
/// future opt-in proxy path.
pub struct StandardAdapter {
    client: reqwest::Client,
    base: String,
}

impl Default for StandardAdapter {
    fn default() -> Self {
        Self::new("https://api.openai.com/v1")
    }
}

impl StandardAdapter {
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
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("valid client"),
            base,
        }
    }

    async fn validate(&self, secret: &secrecy::SecretString) -> Result<(), ProviderError> {
        let response = self
            .client
            .get(format!("{}/models", self.base))
            .bearer_auth(secret.expose_secret())
            .send()
            .await
            .map_err(network_error)?;
        map_status(response.status().as_u16())
    }
}

#[async_trait]
impl ProviderAdapter for StandardAdapter {
    fn manifest(&self) -> ProviderManifest {
        crate::manifest()
    }

    fn supported_auth_schemes(&self) -> Vec<AuthScheme> {
        vec![AuthScheme::ApiKey]
    }

    async fn begin_auth(&self, request: BeginAuthRequest) -> Result<AuthChallenge, ProviderError> {
        if request.auth_scheme != AuthScheme::ApiKey {
            return Err(ProviderError::CapabilityUnavailable);
        }
        Ok(AuthChallenge::SecretInput {
            challenge_id: String::new(),
            label: "OpenAI API Key".into(),
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
        self.validate(&secret).await?;
        let reference = secrets
            .put(
                "io.github.llmmeter.openai",
                &format!("connection_{}", Uuid::new_v4()),
                secret,
            )
            .await?;
        Ok(ConnectionIdentity {
            external_id: "openai-standard-api-key".into(),
            display_name: Some("OpenAI Platform API".into()),
            credential_ref: Some(reference),
            settings: None,
        })
    }

    async fn probe_capabilities(
        &self,
        connection: &ConnectionContext,
    ) -> Result<CapabilitySnapshot, ProviderError> {
        self.validate(secret(connection)?).await?;
        Ok(CapabilitySnapshot {
            connection_id: connection.connection.id,
            capabilities: Capabilities::empty(),
            observed_at: Utc::now(),
        })
    }

    async fn sync(
        &self,
        connection: &ConnectionContext,
        _cursor: Option<SyncCursor>,
    ) -> Result<SyncBatch, ProviderError> {
        Ok(SyncBatch {
            capability_snapshot: Some(self.probe_capabilities(connection).await?),
            provider_timestamp: Some(Utc::now()),
            ..Default::default()
        })
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

fn secret(connection: &ConnectionContext) -> Result<&secrecy::SecretString, ProviderError> {
    connection
        .auth_secret
        .as_ref()
        .ok_or(ProviderError::AuthenticationRequired)
}

fn map_status(status: u16) -> Result<(), ProviderError> {
    match status {
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
    #[should_panic(expected = "require HTTPS")]
    fn remote_plain_http_is_rejected() {
        let _ = StandardAdapter::new("http://api.example.com/v1");
    }

    #[test]
    fn permission_mapping_is_explicit() {
        assert!(matches!(
            map_status(401),
            Err(ProviderError::AuthenticationRequired)
        ));
        assert!(matches!(
            map_status(403),
            Err(ProviderError::PermissionDenied)
        ));
        assert!(matches!(
            map_status(429),
            Err(ProviderError::RateLimited { .. })
        ));
    }
}
