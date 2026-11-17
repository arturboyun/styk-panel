CREATE TABLE users (
    id              BIGSERIAL PRIMARY KEY,
    username        TEXT NOT NULL UNIQUE,
    password_hash   TEXT NOT NULL,
    -- AES-256-GCM(nonce || ciphertext) of the base32 TOTP secret.
    totp_secret_enc BYTEA,
    totp_enabled    BOOLEAN NOT NULL DEFAULT FALSE,
    disabled        BOOLEAN NOT NULL DEFAULT FALSE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Extra ways to sign in as a user (Telegram Login for now).
CREATE TABLE user_identities (
    id          BIGSERIAL PRIMARY KEY,
    user_id     BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider    TEXT NOT NULL,
    subject     TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (provider, subject)
);

CREATE TABLE recovery_codes (
    id         BIGSERIAL PRIMARY KEY,
    user_id    BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash  TEXT NOT NULL,
    used_at    TIMESTAMPTZ
);

-- stage: 'pending_totp' (password ok, waiting for code), 'totp_setup', 'active'.
CREATE TABLE sessions (
    token_hash  TEXT PRIMARY KEY,
    user_id     BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    stage       TEXT NOT NULL,
    method      TEXT NOT NULL,
    ip          TEXT,
    user_agent  TEXT,
    sudo_until  TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen   TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at  TIMESTAMPTZ NOT NULL
);
CREATE INDEX sessions_user_idx ON sessions (user_id);

CREATE TABLE audit_log (
    id        BIGSERIAL PRIMARY KEY,
    ts        TIMESTAMPTZ NOT NULL DEFAULT now(),
    user_id   BIGINT REFERENCES users(id) ON DELETE SET NULL,
    username  TEXT,
    action    TEXT NOT NULL,
    target    TEXT,
    ip        TEXT,
    ok        BOOLEAN NOT NULL,
    details   JSONB NOT NULL DEFAULT '{}'
);
CREATE INDEX audit_log_ts_idx ON audit_log (ts DESC);

CREATE TABLE metrics_host (
    resolution  TEXT NOT NULL,          -- '1m' | '1h'
    ts          TIMESTAMPTZ NOT NULL,
    cpu_pct     REAL NOT NULL,
    mem_used    BIGINT NOT NULL,
    mem_total   BIGINT NOT NULL,
    swap_used   BIGINT NOT NULL,
    load1       REAL NOT NULL,
    disk_used   BIGINT NOT NULL,
    disk_total  BIGINT NOT NULL,
    disk_read_bps   REAL NOT NULL,
    disk_write_bps  REAL NOT NULL,
    net_rx_bps  REAL NOT NULL,
    net_tx_bps  REAL NOT NULL,
    PRIMARY KEY (resolution, ts)
);
CREATE INDEX metrics_host_ts_brin ON metrics_host USING BRIN (ts);

CREATE TABLE metrics_container (
    resolution  TEXT NOT NULL,
    ts          TIMESTAMPTZ NOT NULL,
    name        TEXT NOT NULL,
    project     TEXT,
    cpu_pct     REAL NOT NULL,
    mem_used    BIGINT NOT NULL,
    PRIMARY KEY (resolution, name, ts)
);
CREATE INDEX metrics_container_ts_brin ON metrics_container USING BRIN (ts);
