CREATE TABLE IF NOT EXISTS connection_settings (
  connection_id TEXT PRIMARY KEY,
  schema_version INTEGER NOT NULL,
  settings_json TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY(connection_id) REFERENCES connections(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS usage_events (
  id TEXT PRIMARY KEY,
  connection_id TEXT NOT NULL,
  external_id TEXT NOT NULL,
  occurred_at TEXT NOT NULL,
  observed_at TEXT NOT NULL,
  model TEXT,
  input_tokens INTEGER,
  cached_input_tokens INTEGER,
  output_tokens INTEGER,
  reasoning_tokens INTEGER,
  total_tokens INTEGER,
  request_count INTEGER NOT NULL DEFAULT 1,
  actual_charge_value TEXT,
  actual_charge_unit TEXT,
  upstream_charge_value TEXT,
  upstream_charge_unit TEXT,
  estimated_charge_value TEXT,
  estimated_charge_unit TEXT,
  credit_used_value TEXT,
  provenance TEXT NOT NULL,
  source_event TEXT NOT NULL,
  dimensions_json TEXT NOT NULL,
  UNIQUE(connection_id, source_event, external_id),
  FOREIGN KEY(connection_id) REFERENCES connections(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_usage_events_connection_time
  ON usage_events(connection_id, occurred_at);
CREATE INDEX IF NOT EXISTS idx_usage_events_connection_model_time
  ON usage_events(connection_id, model, occurred_at);
