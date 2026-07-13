CREATE TABLE IF NOT EXISTS proxy_credentials (
  id TEXT PRIMARY KEY,
  connection_id TEXT NOT NULL,
  display_name TEXT NOT NULL,
  token_hash TEXT NOT NULL UNIQUE,
  token_prefix TEXT NOT NULL,
  created_at TEXT NOT NULL,
  last_used_at TEXT,
  disabled_at TEXT,
  FOREIGN KEY(connection_id) REFERENCES connections(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_proxy_credentials_connection
  ON proxy_credentials(connection_id, created_at);
