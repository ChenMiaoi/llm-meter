use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::config::Config;
use chrono::Utc;
use llm_meter_core::{
    ConnectionContext, ConnectionStatus, ProviderAdapter, ProviderError, SecretStore,
};
use llm_meter_storage::Repository;
use tokio::sync::Notify;
use uuid::Uuid;

pub struct ProviderRuntime {
    adapters: HashMap<(String, String), Arc<dyn ProviderAdapter>>,
    pub(crate) repo: Arc<Repository>,
    secrets: Arc<dyn SecretStore>,
    running: Mutex<HashSet<Uuid>>,
    pending_auth: tokio::sync::Mutex<HashMap<String, PendingAuth>>,
    config: Arc<Config>,
}

struct PendingAuth {
    provider_id: String,
    connection_type: String,
    display_name: String,
}

impl ProviderRuntime {
    pub fn new(repo: Arc<Repository>, secrets: Arc<dyn SecretStore>) -> Self {
        Self::with_config(repo, secrets, Arc::new(Config::default()))
    }

    pub fn with_config(
        repo: Arc<Repository>,
        secrets: Arc<dyn SecretStore>,
        config: Arc<Config>,
    ) -> Self {
        Self {
            adapters: HashMap::new(),
            repo,
            secrets,
            running: Mutex::new(HashSet::new()),
            pending_auth: tokio::sync::Mutex::new(HashMap::new()),
            config,
        }
    }

    pub async fn begin_auth(
        &self,
        request: llm_meter_core::BeginAuthRequest,
        provider_id: &str,
    ) -> Result<llm_meter_core::AuthChallenge, ProviderError> {
        let adapter = self
            .adapters
            .get(&(provider_id.into(), request.connection_type.clone()))
            .cloned()
            .ok_or(ProviderError::CapabilityUnavailable)?;
        let connection_type = request.connection_type.clone();
        let display_name = request.display_name.clone();
        let mut challenge = adapter.begin_auth(request).await?;
        let challenge_id = match &mut challenge {
            llm_meter_core::AuthChallenge::Browser { state, .. } => state.clone(),
            llm_meter_core::AuthChallenge::DeviceCode { state, .. } => state.clone(),
            llm_meter_core::AuthChallenge::SecretInput { challenge_id, .. } => {
                *challenge_id = Uuid::new_v4().to_string();
                challenge_id.clone()
            }
            llm_meter_core::AuthChallenge::Complete => Uuid::new_v4().to_string(),
        };
        self.pending_auth.lock().await.insert(
            challenge_id,
            PendingAuth {
                provider_id: provider_id.into(),
                connection_type,
                display_name,
            },
        );
        Ok(challenge)
    }

    pub async fn complete_auth(
        &self,
        challenge_id: &str,
        secret: Option<secrecy::SecretString>,
    ) -> Result<llm_meter_core::Connection, ProviderError> {
        let pending = self
            .pending_auth
            .lock()
            .await
            .remove(challenge_id)
            .ok_or(ProviderError::AuthenticationRequired)?;
        let adapter = self
            .adapters
            .get(&(pending.provider_id.clone(), pending.connection_type.clone()))
            .cloned()
            .ok_or(ProviderError::CapabilityUnavailable)?;
        let identity = adapter
            .complete_auth(
                llm_meter_core::CompleteAuthRequest {
                    challenge_state: Some(challenge_id.into()),
                    secret,
                },
                self.secrets.as_ref(),
            )
            .await?;
        let credential = identity.credential_ref.clone();
        let now = Utc::now();
        let connection = llm_meter_core::Connection {
            id: Uuid::new_v4(),
            provider_id: pending.provider_id,
            connection_type: pending.connection_type,
            display_name: identity.display_name.unwrap_or(pending.display_name),
            account_external_id: Some(identity.external_id),
            status: ConnectionStatus::Ready,
            credential_ref_id: identity.credential_ref.map(|r| r.id),
            created_at: now,
            updated_at: now,
            last_success_at: None,
            last_error_code: None,
            disabled_at: None,
        };
        if let Err(error) = self
            .repo
            .add_authenticated_connection(&connection, credential.as_ref())
        {
            if let Some(reference) = &credential {
                let _ = self.secrets.delete(reference).await;
            }
            return Err(internal(error));
        }
        crate::alerts::install_defaults(&self.repo, connection.id).map_err(internal)?;
        Ok(connection)
    }

    pub fn register(
        &mut self,
        provider: &str,
        connection_type: &str,
        adapter: Arc<dyn ProviderAdapter>,
    ) {
        self.adapters
            .insert((provider.into(), connection_type.into()), adapter);
    }

    pub fn stale_after_seconds(&self) -> i64 {
        self.config.stale_after_seconds
    }

    pub fn provider_manifests(&self) -> Vec<llm_meter_core::ProviderManifest> {
        self.adapters
            .values()
            .map(|adapter| adapter.manifest())
            .map(|manifest| (manifest.provider_id.clone(), manifest))
            .collect::<BTreeMap<_, _>>()
            .into_values()
            .collect()
    }

    pub async fn sync(&self, id: Uuid, manual: bool) -> Result<(), ProviderError> {
        self.reserve_sync(id, manual)?;
        self.sync_reserved(id, manual).await
    }

    /// A user-initiated refresh bypasses the local debounce. If an automatic
    /// refresh is already in flight, wait for it and accept its fresh result;
    /// never run two requests for the same connection concurrently.
    pub async fn force_sync(&self, id: Uuid) -> Result<(), ProviderError> {
        let initial_success = self
            .repo
            .connection(id)
            .map_err(internal)?
            .and_then(|connection| connection.last_success_at);
        let mut waited_for_existing = false;
        for _ in 0..80 {
            let reserved = {
                let mut running = self
                    .running
                    .lock()
                    .map_err(|_| ProviderError::Internal("scheduler lock poisoned".into()))?;
                running.insert(id)
            };
            if reserved {
                let current_success = self
                    .repo
                    .connection(id)
                    .map_err(internal)?
                    .and_then(|connection| connection.last_success_at);
                if waited_for_existing && current_success != initial_success {
                    if let Ok(mut running) = self.running.lock() {
                        running.remove(&id);
                    }
                    return Ok(());
                }
                return self.sync_reserved(id, true).await;
            }
            waited_for_existing = true;
            tokio::time::sleep(Duration::from_millis(125)).await;
        }
        Err(ProviderError::Timeout)
    }

    /// Accept a refresh without tying the IPC response to provider network latency.
    pub fn request_sync(self: &Arc<Self>, id: Uuid, manual: bool) -> Result<(), ProviderError> {
        self.reserve_sync(id, manual)?;
        let runtime = self.clone();
        tokio::spawn(async move {
            let _ = runtime.sync_reserved(id, manual).await;
        });
        Ok(())
    }

    fn reserve_sync(&self, id: Uuid, manual: bool) -> Result<(), ProviderError> {
        if manual
            && self
                .repo
                .sync_last_attempt(id, "default")
                .map_err(internal)?
                .is_some_and(|last| {
                    Utc::now().signed_duration_since(last).num_seconds()
                        < self.config.manual_refresh_min_seconds as i64
                })
        {
            return Err(ProviderError::RateLimited {
                retry_at: Some(
                    Utc::now()
                        + chrono::Duration::seconds(self.config.manual_refresh_min_seconds as i64),
                ),
            });
        }
        if !manual {
            let (retry_at, _) = self
                .repo
                .sync_retry_state(id, "default")
                .map_err(internal)?;
            if retry_at.is_some_and(|retry| retry > Utc::now()) {
                return Err(ProviderError::RateLimited { retry_at });
            }
        }
        {
            let mut running = self
                .running
                .lock()
                .map_err(|_| ProviderError::Internal("scheduler lock poisoned".into()))?;
            if !running.insert(id) {
                return Err(ProviderError::RateLimited { retry_at: None });
            }
        }
        Ok(())
    }

    async fn sync_reserved(&self, id: Uuid, manual: bool) -> Result<(), ProviderError> {
        let started = std::time::Instant::now();
        let provider_id = self
            .repo
            .connection(id)
            .ok()
            .flatten()
            .map(|value| value.provider_id)
            .unwrap_or_else(|| "unknown".into());
        let result = self.sync_inner(id, manual).await;
        let latency_ms = started.elapsed().as_millis() as u64;
        crate::telemetry::record_sync(
            latency_ms,
            result.is_ok(),
            matches!(&result, Err(ProviderError::RateLimited { .. })),
        );
        tracing::info!(
            component = "provider_runtime",
            provider_id,
            connection_id = %id,
            operation = "sync",
            latency_ms,
            result = if result.is_ok() { "success" } else { "failure" },
            error_code = result.as_ref().err().map(ProviderError::code),
            "provider synchronization completed"
        );
        if let Ok(mut running) = self.running.lock() {
            running.remove(&id);
        }
        result
    }

    async fn sync_inner(&self, id: Uuid, _manual: bool) -> Result<(), ProviderError> {
        let connection = self
            .repo
            .connection(id)
            .map_err(internal)?
            .ok_or_else(|| ProviderError::Internal("connection not found".into()))?;
        if connection.status == ConnectionStatus::Disabled {
            return Err(ProviderError::CapabilityUnavailable);
        }
        let adapter = self
            .adapters
            .get(&(
                connection.provider_id.clone(),
                connection.connection_type.clone(),
            ))
            .cloned()
            .ok_or(ProviderError::CapabilityUnavailable)?;
        let credential_ref = match connection.credential_ref_id {
            Some(id) => self.repo.credential_ref(id).map_err(internal)?,
            None => None,
        };
        let auth_secret = match &credential_ref {
            Some(reference) => Some(self.secrets.get(reference).await?),
            None => None,
        };
        let context = ConnectionContext {
            connection,
            credential_ref,
            auth_secret,
        };
        let connection_name = context.connection.display_name.clone();
        self.repo
            .set_connection_status(id, ConnectionStatus::Syncing)
            .map_err(internal)?;
        let cursor = self.repo.sync_cursor(id, "default").map_err(internal)?;
        let outcome = tokio::time::timeout(Duration::from_secs(90), adapter.sync(&context, cursor))
            .await
            .map_err(|_| ProviderError::Timeout)
            .and_then(|value| value);
        match outcome {
            Ok(batch) => {
                let write_started = std::time::Instant::now();
                self.repo
                    .commit_sync_batch(id, "default", &batch)
                    .map_err(internal)?;
                crate::telemetry::record_sqlite_write(write_started.elapsed().as_millis() as u64);
                if let Ok(changed) = crate::alerts::evaluate(&self.repo, id) {
                    crate::telemetry::record_alerts(changed.len());
                    let name = connection_name.clone();
                    tokio::task::spawn_blocking(move || {
                        crate::notifications::deliver(&name, &changed)
                    });
                }
                Ok(())
            }
            Err(error) => {
                let error_count = self
                    .repo
                    .sync_retry_state(id, "default")
                    .map(|(_, count)| count)
                    .unwrap_or(0);
                let (status, retry_at) = error_status(&error, error_count);
                let _ = self
                    .repo
                    .mark_sync_error(id, "default", status, error.code(), retry_at);
                if let Ok(changed) = crate::alerts::evaluate(&self.repo, id) {
                    crate::telemetry::record_alerts(changed.len());
                    let name = connection_name;
                    tokio::task::spawn_blocking(move || {
                        crate::notifications::deliver(&name, &changed)
                    });
                }
                Err(error)
            }
        }
    }

    pub async fn remove(&self, id: Uuid) -> Result<(), ProviderError> {
        let connection = self
            .repo
            .connection(id)
            .map_err(internal)?
            .ok_or_else(|| ProviderError::Internal("connection not found".into()))?;
        let credential_ref = connection
            .credential_ref_id
            .and_then(|id| self.repo.credential_ref(id).ok().flatten());
        let fallback_credential = credential_ref.clone();
        let context = ConnectionContext {
            connection,
            credential_ref,
            auth_secret: None,
        };
        if let Some(adapter) = self.adapters.get(&(
            context.connection.provider_id.clone(),
            context.connection.connection_type.clone(),
        )) {
            adapter.disconnect(&context, self.secrets.as_ref()).await?;
        } else if let Some(reference) = &fallback_credential {
            self.secrets.delete(reference).await?;
        }
        self.repo.remove_connection(id).map_err(internal)?;
        Ok(())
    }
}

fn internal(error: impl std::fmt::Display) -> ProviderError {
    ProviderError::Internal(error.to_string())
}

fn error_status(
    error: &ProviderError,
    prior_error_count: u32,
) -> (ConnectionStatus, Option<chrono::DateTime<Utc>>) {
    let exponential = |base_seconds: i64| {
        let multiplier = 1_i64 << prior_error_count.min(8);
        Utc::now() + chrono::Duration::seconds((base_seconds * multiplier).min(3600))
    };
    match error {
        ProviderError::AuthenticationRequired => (ConnectionStatus::AuthRequired, None),
        ProviderError::RateLimited { retry_at } => (
            ConnectionStatus::RateLimited,
            retry_at.or_else(|| Some(exponential(300))),
        ),
        ProviderError::NetworkUnavailable | ProviderError::Timeout => {
            (ConnectionStatus::Offline, Some(exponential(120)))
        }
        _ => (ConnectionStatus::ProviderError, Some(exponential(300))),
    }
}

pub struct Scheduler {
    runtime: Arc<ProviderRuntime>,
    wake: Arc<Notify>,
}

impl Scheduler {
    pub fn new(runtime: Arc<ProviderRuntime>) -> Self {
        Self {
            runtime,
            wake: Arc::new(Notify::new()),
        }
    }

    pub fn wake(&self) {
        self.wake.notify_one();
    }

    pub async fn run(&self) {
        let mut last_prune = Utc::now() - chrono::Duration::days(1);
        loop {
            let connections = self.runtime.repo.list_connections().unwrap_or_default();
            for connection in connections {
                if matches!(
                    connection.status,
                    ConnectionStatus::Disabled
                        | ConnectionStatus::AuthRequired
                        | ConnectionStatus::Connecting
                ) {
                    continue;
                }
                let due = connection.last_success_at.is_none_or(|last| {
                    Utc::now().signed_duration_since(last).num_seconds()
                        >= self
                            .runtime
                            .config
                            .interval_for(&connection.connection_type)
                            as i64
                });
                if due {
                    let runtime = self.runtime.clone();
                    tokio::spawn(async move {
                        let _ = runtime.sync(connection.id, false).await;
                    });
                }
            }
            if Utc::now().signed_duration_since(last_prune).num_hours() >= 24 {
                let retention = &self.runtime.config.retention;
                let _ = self.runtime.repo.prune(
                    Utc::now(),
                    retention.raw_days,
                    retention.hourly_days,
                    retention.provider_events_days,
                );
                last_prune = Utc::now();
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(self.runtime.config.scheduler_tick_seconds)) => {},
                _ = self.wake.notified() => {},
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_meter_core::{AuthScheme, BeginAuthRequest};
    use llm_meter_provider_testkit::MockProvider;
    use llm_meter_secret_store::MemorySecretStore;
    #[test]
    fn bounded_error_mapping() {
        let (status, retry) = error_status(&ProviderError::RateLimited { retry_at: None }, 0);
        assert_eq!(status, ConnectionStatus::RateLimited);
        assert!(retry.is_some());
    }

    #[tokio::test]
    async fn auth_workflow_creates_connection_without_secret_material() {
        let repo = Arc::new(Repository::in_memory().unwrap());
        let mut runtime =
            ProviderRuntime::new(repo.clone(), Arc::new(MemorySecretStore::default()));
        runtime.register("mock", "fixture", Arc::new(MockProvider));
        let challenge = runtime
            .begin_auth(
                BeginAuthRequest {
                    connection_type: "fixture".into(),
                    auth_scheme: AuthScheme::Manual,
                    display_name: "Test Mock".into(),
                },
                "mock",
            )
            .await
            .unwrap();
        assert!(matches!(challenge, llm_meter_core::AuthChallenge::Complete));
        let id = runtime
            .pending_auth
            .lock()
            .await
            .keys()
            .next()
            .unwrap()
            .clone();
        let connection = runtime.complete_auth(&id, None).await.unwrap();
        assert_eq!(connection.provider_id, "mock");
        assert_eq!(repo.list_connections().unwrap().len(), 1);
    }
}
