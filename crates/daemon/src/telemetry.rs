use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};

static SYNC_DURATION_MS: AtomicU64 = AtomicU64::new(0);
static SYNC_SUCCESS_TOTAL: AtomicU64 = AtomicU64::new(0);
static SYNC_FAILURE_TOTAL: AtomicU64 = AtomicU64::new(0);
static PROVIDER_RATE_LIMIT_TOTAL: AtomicU64 = AtomicU64::new(0);
static IPC_REQUEST_DURATION_MS: AtomicU64 = AtomicU64::new(0);
static SQLITE_WRITE_DURATION_MS: AtomicU64 = AtomicU64::new(0);
static SNAPSHOT_AGE_SECONDS: AtomicU64 = AtomicU64::new(0);
static ALERT_TRIGGER_TOTAL: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Serialize)]
pub struct OperationalMetrics {
    pub sync_duration_ms: u64,
    pub sync_success_total: u64,
    pub sync_failure_total: u64,
    pub provider_rate_limit_total: u64,
    pub ipc_request_duration_ms: u64,
    pub sqlite_write_duration_ms: u64,
    pub snapshot_age_seconds: u64,
    pub alert_trigger_total: u64,
}

pub fn record_sync(duration_ms: u64, success: bool, rate_limited: bool) {
    SYNC_DURATION_MS.store(duration_ms, Ordering::Relaxed);
    if success {
        SYNC_SUCCESS_TOTAL.fetch_add(1, Ordering::Relaxed);
    } else {
        SYNC_FAILURE_TOTAL.fetch_add(1, Ordering::Relaxed);
    }
    if rate_limited {
        PROVIDER_RATE_LIMIT_TOTAL.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn record_ipc(duration_ms: u64) {
    IPC_REQUEST_DURATION_MS.store(duration_ms, Ordering::Relaxed);
}

pub fn record_sqlite_write(duration_ms: u64) {
    SQLITE_WRITE_DURATION_MS.store(duration_ms, Ordering::Relaxed);
}

pub fn record_snapshot_age(seconds: u64) {
    SNAPSHOT_AGE_SECONDS.store(seconds, Ordering::Relaxed);
}

pub fn record_alerts(count: usize) {
    ALERT_TRIGGER_TOTAL.fetch_add(count as u64, Ordering::Relaxed);
}

pub fn snapshot() -> OperationalMetrics {
    OperationalMetrics {
        sync_duration_ms: SYNC_DURATION_MS.load(Ordering::Relaxed),
        sync_success_total: SYNC_SUCCESS_TOTAL.load(Ordering::Relaxed),
        sync_failure_total: SYNC_FAILURE_TOTAL.load(Ordering::Relaxed),
        provider_rate_limit_total: PROVIDER_RATE_LIMIT_TOTAL.load(Ordering::Relaxed),
        ipc_request_duration_ms: IPC_REQUEST_DURATION_MS.load(Ordering::Relaxed),
        sqlite_write_duration_ms: SQLITE_WRITE_DURATION_MS.load(Ordering::Relaxed),
        snapshot_age_seconds: SNAPSHOT_AGE_SECONDS.load(Ordering::Relaxed),
        alert_trigger_total: ALERT_TRIGGER_TOTAL.load(Ordering::Relaxed),
    }
}
