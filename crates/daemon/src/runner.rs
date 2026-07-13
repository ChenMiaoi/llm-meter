use crate::{
    ipc::Server,
    scheduler::{ProviderRuntime, Scheduler},
    socket_path,
};
use llm_meter_provider_kimi::subscription::SubscriptionAdapter as KimiSubscriptionAdapter;
use llm_meter_provider_openai::{
    platform::AdminAdapter, standard::StandardAdapter, subscription::SubscriptionAdapter,
};
use llm_meter_provider_relay::RelayAdapter;
use llm_meter_secret_store::NativeSecretStore;
use llm_meter_storage::Repository;
use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    io,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tracing_subscriber::fmt::writer::MakeWriterExt;

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let home = crate::app_home()?;
    let data = home.join("data");
    let logs = home.join("logs");
    prepare_directories(&home, &data, &logs)?;
    migrate_legacy_layout(&home, &data)?;
    let log = open_log(&logs.join("llm-meterd.log"))?;
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .with_writer(Arc::new(log).and(std::io::stdout))
        .init();
    let database = data.join("llm-meter.sqlite3");
    let repo = Arc::new(Repository::open(&database)?);
    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o600))?;
    let socket = socket_path()?;
    let config = Arc::new(crate::config::Config::load(&crate::config::Config::path()?)?);
    let retention = &config.retention;
    repo.prune(
        chrono::Utc::now(),
        retention.raw_days,
        retention.hourly_days,
        retention.provider_events_days,
    )?;
    let mut runtime =
        ProviderRuntime::with_config(repo.clone(), Arc::new(NativeSecretStore), config);
    runtime.register(
        "openai",
        "chatgpt_subscription",
        Arc::new(SubscriptionAdapter::default()),
    );
    runtime.register(
        "kimi",
        "kimi_code_subscription",
        Arc::new(KimiSubscriptionAdapter::default()),
    );
    runtime.register(
        "openai",
        "platform_admin",
        Arc::new(AdminAdapter::default()),
    );
    runtime.register(
        "openai",
        "platform_standard",
        Arc::new(StandardAdapter::default()),
    );
    let relay = Arc::new(RelayAdapter::default());
    runtime.register("relay", "new_api", relay.clone());
    runtime.register("relay", "openrouter", relay.clone());
    runtime.register("relay", "openai_compatible_proxy", relay);
    let runtime = Arc::new(runtime);
    for connection in repo.list_connections()? {
        if connection.connection_type == "openai_compatible_proxy"
            && connection.disabled_at.is_none()
        {
            let _ = runtime.start_proxy(connection.id).await;
        }
    }
    let scheduler = Scheduler::new(runtime.clone());
    tokio::spawn(async move { scheduler.run().await });
    tokio::spawn(monitor_local_codex());
    tracing::info!(component="daemon",operation="startup",socket=%socket.display(),"llm-meter daemon ready");
    Server::with_runtime(repo, runtime).serve(&socket).await?;
    Ok(())
}

async fn monitor_local_codex() {
    let mut previous_sessions = BTreeSet::<(String, String)>::new();
    let mut last_summary = Instant::now();
    loop {
        let started = Instant::now();
        match tokio::task::spawn_blocking(crate::local_codex::snapshot).await {
            Ok(snapshot) => {
                let sessions = snapshot
                    .active_sessions
                    .iter()
                    .map(|session| (session.id.clone(), session.model.clone()))
                    .collect::<BTreeSet<_>>();
                let models = snapshot
                    .models
                    .iter()
                    .map(|usage| usage.model.as_str())
                    .collect::<Vec<_>>()
                    .join(",");
                if sessions != previous_sessions {
                    tracing::info!(
                        component = "local_codex",
                        operation = "discovery",
                        active_sessions = sessions.len(),
                        models,
                        "local Codex sessions changed"
                    );
                    previous_sessions = sessions;
                }
                if last_summary.elapsed() >= Duration::from_secs(30) {
                    tracing::info!(
                        component = "local_codex",
                        operation = "refresh",
                        active_sessions = snapshot.active_sessions.len(),
                        models,
                        today_tokens = snapshot.today_tokens,
                        today_estimated_cost_usd = snapshot
                            .today_estimated_cost_usd
                            .map(|value| value.to_string()),
                        latency_ms = started.elapsed().as_millis() as u64,
                        "local Codex refresh completed"
                    );
                    last_summary = Instant::now();
                }
            }
            Err(error) => tracing::warn!(
                component = "local_codex",
                operation = "refresh",
                error = %error,
                "local Codex refresh task failed"
            ),
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

fn prepare_directories(home: &Path, data: &Path, logs: &Path) -> io::Result<()> {
    for path in [home, data, logs] {
        std::fs::create_dir_all(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn open_log(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
}

/// Move the original XDG layout on first launch. SQLite's WAL and SHM files
/// move together with the main database so no committed data is discarded.
fn migrate_legacy_layout(home: &Path, data: &Path) -> io::Result<()> {
    let Some(user_home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Ok(());
    };
    let old_data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| user_home.join(".local/share"))
        .join("llm-meter");
    for name in [
        "llm-meter.sqlite3",
        "llm-meter.sqlite3-wal",
        "llm-meter.sqlite3-shm",
    ] {
        let source = old_data.join(name);
        let target = data.join(name);
        if source.is_file() && !target.exists() {
            move_file(&source, &target)?;
        }
    }
    let old_config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| user_home.join(".config"))
        .join("llm-meter/config.toml");
    let new_config = home.join("config.toml");
    if old_config.is_file() && !new_config.exists() {
        move_file(&old_config, &new_config)?;
        std::fs::set_permissions(&new_config, std::fs::Permissions::from_mode(0o600))?;
    }
    let _ = std::fs::remove_dir(&old_data);
    if let Some(parent) = old_config.parent() {
        let _ = std::fs::remove_dir(parent);
    }
    Ok(())
}

fn move_file(source: &Path, target: &Path) -> io::Result<()> {
    match std::fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(error) if error.raw_os_error() == Some(libc_exdev()) => {
            std::fs::copy(source, target)?;
            std::fs::remove_file(source)
        }
        Err(error) => Err(error),
    }
}

fn libc_exdev() -> i32 {
    18
}
