-- Create table for BFF session storage.
CREATE TABLE bff_sessions (
    session_id   TEXT PRIMARY KEY,
    payload      BYTEA NOT NULL,
    expires_at   TIMESTAMPTZ NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_bff_sessions_expires_at ON bff_sessions (expires_at);
