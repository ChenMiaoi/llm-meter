CREATE TABLE IF NOT EXISTS providers (
  id TEXT PRIMARY KEY, display_name TEXT NOT NULL, adapter_version TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS credential_refs (
  id TEXT PRIMARY KEY, backend TEXT NOT NULL, service_name TEXT NOT NULL,
  secret_key TEXT NOT NULL, created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS connections (
  id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, connection_type TEXT NOT NULL,
  display_name TEXT NOT NULL, account_external_id TEXT, status TEXT NOT NULL,
  credential_ref_id TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
  last_success_at TEXT, last_error_code TEXT, disabled_at TEXT,
  FOREIGN KEY (credential_ref_id) REFERENCES credential_refs(id)
);
CREATE TABLE IF NOT EXISTS accounts (
  id TEXT PRIMARY KEY, connection_id TEXT NOT NULL, external_id TEXT NOT NULL,
  display_name TEXT, account_type TEXT, UNIQUE(connection_id, external_id),
  FOREIGN KEY(connection_id) REFERENCES connections(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS products (
  id TEXT PRIMARY KEY, connection_id TEXT NOT NULL, product_key TEXT NOT NULL,
  display_name TEXT, UNIQUE(connection_id, product_key),
  FOREIGN KEY(connection_id) REFERENCES connections(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS connection_capabilities (
  connection_id TEXT PRIMARY KEY, flags INTEGER NOT NULL, observed_at TEXT NOT NULL,
  FOREIGN KEY(connection_id) REFERENCES connections(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS metric_samples (
  id TEXT PRIMARY KEY, connection_id TEXT NOT NULL, metric_key TEXT NOT NULL,
  value TEXT NOT NULL, unit TEXT NOT NULL, scope TEXT NOT NULL, period_start TEXT,
  period_end TEXT, observed_at TEXT NOT NULL, provenance TEXT NOT NULL,
  dimensions_json TEXT NOT NULL, source_metric TEXT NOT NULL, dedup_key TEXT NOT NULL UNIQUE,
  FOREIGN KEY(connection_id) REFERENCES connections(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_metrics_connection_time ON metric_samples(connection_id, observed_at);
CREATE INDEX IF NOT EXISTS idx_metrics_key_period ON metric_samples(metric_key, period_start, period_end);
CREATE TABLE IF NOT EXISTS quota_windows (
  id TEXT PRIMARY KEY, connection_id TEXT NOT NULL, provider_limit_id TEXT NOT NULL,
  display_name TEXT, window_kind TEXT NOT NULL, window_start TEXT, window_end TEXT,
  resets_at TEXT, used_ratio TEXT, remaining_ratio TEXT, used_value TEXT, limit_value TEXT,
  unit TEXT, provenance TEXT NOT NULL, observed_at TEXT NOT NULL,
  UNIQUE(connection_id, provider_limit_id),
  FOREIGN KEY(connection_id) REFERENCES connections(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS budgets (
  id TEXT PRIMARY KEY, connection_id TEXT NOT NULL, amount TEXT NOT NULL,
  currency TEXT NOT NULL, period TEXT NOT NULL, warning_ratio TEXT NOT NULL,
  critical_ratio TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1, UNIQUE(connection_id),
  FOREIGN KEY(connection_id) REFERENCES connections(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS alerts (
  id TEXT PRIMARY KEY, connection_id TEXT NOT NULL, kind TEXT NOT NULL,
  threshold TEXT NOT NULL, state TEXT NOT NULL, last_triggered_at TEXT,
  suppressed_until TEXT, FOREIGN KEY(connection_id) REFERENCES connections(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS sync_states (
  connection_id TEXT NOT NULL, stream_name TEXT NOT NULL, cursor TEXT,
  last_attempt_at TEXT, last_success_at TEXT, next_retry_at TEXT,
  error_count INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(connection_id, stream_name),
  FOREIGN KEY(connection_id) REFERENCES connections(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS provider_events (
  id TEXT PRIMARY KEY, connection_id TEXT NOT NULL, event_type TEXT NOT NULL,
  observed_at TEXT NOT NULL, summary_json TEXT NOT NULL,
  FOREIGN KEY(connection_id) REFERENCES connections(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS schema_migrations (
  version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL
);
