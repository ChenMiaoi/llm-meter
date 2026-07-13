CREATE TABLE IF NOT EXISTS rate_limit_reset_credit_summaries (
  connection_id TEXT PRIMARY KEY,
  available_count INTEGER NOT NULL,
  details_available INTEGER NOT NULL DEFAULT 0,
  observed_at TEXT NOT NULL,
  FOREIGN KEY(connection_id) REFERENCES connections(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS rate_limit_reset_credits (
  id TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  reset_type TEXT NOT NULL,
  status TEXT NOT NULL,
  granted_at TEXT NOT NULL,
  expires_at TEXT,
  title TEXT,
  description TEXT,
  PRIMARY KEY(connection_id, id),
  FOREIGN KEY(connection_id) REFERENCES connections(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_reset_credits_expiry
  ON rate_limit_reset_credits(connection_id, expires_at);
