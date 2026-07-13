use std::{collections::HashMap, sync::Mutex};

use async_trait::async_trait;
use chrono::Utc;
use llm_meter_core::{CredentialRef, ProviderError, SecretStore};
use secrecy::{ExposeSecret, SecretString};
use uuid::Uuid;

/// Secret Service / Keychain / Credential Manager backed store. Only opaque
/// references returned from this type are persisted by the repository.
#[derive(Debug, Default, Clone)]
pub struct NativeSecretStore;

impl NativeSecretStore {
    pub async fn availability(&self) -> &'static str {
        tokio::task::spawn_blocking(|| {
            let entry = match keyring::Entry::new("io.github.llmmeter.probe", "availability") {
                Ok(value) => value,
                Err(_) => return "unavailable",
            };
            match entry.get_password() {
                Ok(secret) => {
                    drop(secret);
                    "available"
                }
                Err(keyring::Error::NoEntry) => "available",
                Err(keyring::Error::NoStorageAccess(_)) => "locked",
                Err(_) => "unavailable",
            }
        })
        .await
        .unwrap_or("unavailable")
    }
}

#[async_trait]
impl SecretStore for NativeSecretStore {
    async fn put(
        &self,
        service: &str,
        key: &str,
        secret: SecretString,
    ) -> Result<CredentialRef, ProviderError> {
        let service_owned = service.to_owned();
        let key_owned = key.to_owned();
        tokio::task::spawn_blocking(move || {
            keyring::Entry::new(&service_owned, &key_owned)
                .map_err(safe_keyring)?
                .set_password(secret.expose_secret())
                .map_err(safe_keyring)
        })
        .await
        .map_err(|_| ProviderError::SecretStoreUnavailable)??;
        Ok(CredentialRef {
            id: Uuid::new_v4(),
            backend: backend_name().into(),
            service_name: service.into(),
            secret_key: key.into(),
            created_at: Utc::now(),
        })
    }

    async fn get(&self, r: &CredentialRef) -> Result<SecretString, ProviderError> {
        let r = r.clone();
        tokio::task::spawn_blocking(move || {
            keyring::Entry::new(&r.service_name, &r.secret_key)
                .map_err(safe_keyring)?
                .get_password()
                .map(SecretString::from)
                .map_err(safe_keyring)
        })
        .await
        .map_err(|_| ProviderError::SecretStoreUnavailable)?
    }

    async fn delete(&self, r: &CredentialRef) -> Result<(), ProviderError> {
        let r = r.clone();
        tokio::task::spawn_blocking(move || {
            keyring::Entry::new(&r.service_name, &r.secret_key)
                .map_err(safe_keyring)?
                .delete_credential()
                .map_err(safe_keyring)
        })
        .await
        .map_err(|_| ProviderError::SecretStoreUnavailable)?
    }

    async fn available(&self) -> bool {
        self.availability().await == "available"
    }
}

fn safe_keyring(_: keyring::Error) -> ProviderError {
    ProviderError::SecretStoreUnavailable
}
fn backend_name() -> &'static str {
    if cfg!(target_os = "linux") {
        "secret-service"
    } else if cfg!(target_os = "macos") {
        "keychain"
    } else if cfg!(target_os = "windows") {
        "credential-manager"
    } else {
        "unsupported"
    }
}

/// Test-only/in-process implementation. Its secret map cannot be serialized.
#[derive(Debug, Default)]
pub struct MemorySecretStore {
    values: Mutex<HashMap<(String, String), SecretString>>,
}

#[async_trait]
impl SecretStore for MemorySecretStore {
    async fn put(
        &self,
        service: &str,
        key: &str,
        secret: SecretString,
    ) -> Result<CredentialRef, ProviderError> {
        self.values
            .lock()
            .map_err(|_| ProviderError::Internal("secret lock poisoned".into()))?
            .insert((service.into(), key.into()), secret);
        Ok(CredentialRef {
            id: Uuid::new_v4(),
            backend: "memory".into(),
            service_name: service.into(),
            secret_key: key.into(),
            created_at: Utc::now(),
        })
    }
    async fn get(&self, r: &CredentialRef) -> Result<SecretString, ProviderError> {
        self.values
            .lock()
            .map_err(|_| ProviderError::Internal("secret lock poisoned".into()))?
            .get(&(r.service_name.clone(), r.secret_key.clone()))
            .cloned()
            .ok_or(ProviderError::AuthenticationRequired)
    }
    async fn delete(&self, r: &CredentialRef) -> Result<(), ProviderError> {
        self.values
            .lock()
            .map_err(|_| ProviderError::Internal("secret lock poisoned".into()))?
            .remove(&(r.service_name.clone(), r.secret_key.clone()));
        Ok(())
    }
    async fn available(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn memory_roundtrip() {
        let s = MemorySecretStore::default();
        let r = s
            .put("svc", "key", SecretString::from("never-log-this"))
            .await
            .unwrap();
        assert_eq!(s.get(&r).await.unwrap().expose_secret(), "never-log-this");
        s.delete(&r).await.unwrap();
        assert!(s.get(&r).await.is_err());
    }
}
