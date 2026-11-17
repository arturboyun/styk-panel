-- Long-running operations (agent tasks) with their output.
CREATE TABLE jobs (
    id          BIGSERIAL PRIMARY KEY,
    kind        TEXT NOT NULL,
    target      TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'running', -- running | ok | failed | lost
    user_id     BIGINT REFERENCES users(id) ON DELETE SET NULL,
    username    TEXT,
    agent_task  BIGINT,
    log         TEXT NOT NULL DEFAULT '',
    log_offset  INT NOT NULL DEFAULT 0,
    exit_code   INT,
    started_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ
);
CREATE INDEX jobs_started ON jobs (started_at DESC);
CREATE INDEX jobs_kind_target ON jobs (kind, target, started_at DESC);

CREATE TABLE settings (
    key        TEXT PRIMARY KEY,
    value      JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Built-in rules are code; this table only keeps the user's overrides.
CREATE TABLE alert_rules (
    key         TEXT PRIMARY KEY,
    enabled     BOOLEAN NOT NULL DEFAULT true,
    params      JSONB NOT NULL DEFAULT '{}',
    muted_until TIMESTAMPTZ
);

CREATE TABLE alert_events (
    id               BIGSERIAL PRIMARY KEY,
    rule             TEXT NOT NULL,
    subject          TEXT NOT NULL,
    severity         TEXT NOT NULL, -- info | warning | critical
    status           TEXT NOT NULL, -- firing | resolved
    message          TEXT NOT NULL,
    details          JSONB NOT NULL DEFAULT '{}',
    started_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at      TIMESTAMPTZ,
    acked_by         TEXT,
    acked_at         TIMESTAMPTZ,
    muted_until      TIMESTAMPTZ,
    last_notified_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX alert_events_open ON alert_events (rule, subject) WHERE status = 'firing';
CREATE INDEX alert_events_started ON alert_events (started_at DESC);

CREATE TABLE uptime_checks (
    id         BIGSERIAL PRIMARY KEY,
    url        TEXT NOT NULL UNIQUE,
    name       TEXT,
    source     TEXT NOT NULL DEFAULT 'manual', -- caddy | manual
    enabled    BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE uptime_results (
    check_id   BIGINT NOT NULL REFERENCES uptime_checks(id) ON DELETE CASCADE,
    ts         TIMESTAMPTZ NOT NULL,
    up         BOOLEAN NOT NULL,
    status     INT,
    latency_ms INT,
    error      TEXT,
    PRIMARY KEY (check_id, ts)
);
CREATE INDEX uptime_results_ts ON uptime_results USING BRIN (ts);

-- HMAC needs the key itself, so it is kept encrypted (STYK_SECRET_KEY), never in clear.
CREATE TABLE deploy_hooks (
    project      TEXT PRIMARY KEY,
    secret_enc   BYTEA NOT NULL,
    created_by   TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ
);
