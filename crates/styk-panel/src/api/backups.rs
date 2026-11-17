//! Backups & databases: backup-all status, dump files, Postgres/Redis containers,
//! dumps and restores as jobs, a read-only SQL console, systemd timers.

use std::collections::HashMap;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::Json;
use bollard::container::ListContainersOptions;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use styk_proto::{Method, Task, Timer};

use crate::audit;
use crate::auth::{AuthUser, ClientInfo};
use crate::docker::{container_env, exec};
use crate::error::{agent_error, ApiError, ApiResult};
use crate::jobs;
use crate::settings;
use crate::state::AppState;

const BACKUP_UNIT: &str = "backup-all.service";
const BACKUP_TIMER: &str = "backup-all.timer";
const RESTORE_TEST_UNIT: &str = "backup-restore-test.service";
const RESTORE_TEST_TIMER: &str = "backup-restore-test.timer";
/// Settings key holding the restore-test schedule as the panel last set it.
const RESTORE_TEST_KEY: &str = "restore_test_schedule";
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// Restore test schedule: weekdays plus a UTC time. The timer file ships with Mon 05:00.
#[derive(Serialize, Deserialize, Clone)]
pub struct Schedule {
    enabled: bool,
    days: Vec<String>,
    time: String,
}

impl Default for Schedule {
    fn default() -> Self {
        Schedule {
            enabled: true,
            days: vec!["Mon".into()],
            time: "05:00".into(),
        }
    }
}

impl Schedule {
    /// systemd OnCalendar, e.g. `Mon,Thu *-*-* 05:00:00 UTC`.
    fn on_calendar(&self) -> Result<String, ApiError> {
        let bad = |m: &str| ApiError::BadRequest(m.into());
        let days: Vec<&str> = WEEKDAYS
            .iter()
            .copied()
            .filter(|d| self.days.iter().any(|x| x == d))
            .collect();
        if days.is_empty() || days.len() != self.days.len() {
            return Err(bad("выберите хотя бы один день недели"));
        }
        let (h, m) = self
            .time
            .split_once(':')
            .and_then(|(h, m)| Some((h.parse::<u8>().ok()?, m.parse::<u8>().ok()?)))
            .filter(|&(h, m)| h < 24 && m < 60)
            .ok_or_else(|| bad("время в формате ЧЧ:ММ"))?;
        Ok(format!("{} *-*-* {h:02}:{m:02}:00 UTC", days.join(",")))
    }
}

/// Five agent calls, so one result serves every open tab for 30 s; job ends, schedule
/// changes and unit runs invalidate `backups:`.
pub async fn overview(State(st): State<AppState>, _u: AuthUser) -> ApiResult<Json<Value>> {
    let mut v = st
        .cache
        .get_or("backups:overview", CACHE_TTL, || overview_now(&st))
        .await?;
    let schedule: Schedule = settings::get(&st, RESTORE_TEST_KEY)
        .await
        .unwrap_or_default();
    v["restore_test"]["schedule"] = json!(schedule);
    Ok(Json(v))
}

const CACHE_TTL: Duration = Duration::from_secs(30);

async fn overview_now(st: &AppState) -> ApiResult<Value> {
    let (info, test_info, timers, files, status) = tokio::join!(
        st.agent.call::<Value>(Method::UnitInfo {
            unit: BACKUP_UNIT.into()
        }),
        st.agent.call::<Value>(Method::UnitInfo {
            unit: RESTORE_TEST_UNIT.into()
        }),
        st.agent.call::<Vec<Timer>>(Method::Timers),
        st.agent.call::<Value>(Method::Backups),
        st.agent.call::<Value>(Method::BackupStatus),
    );
    let timers = timers.map_err(agent_error)?;
    let find = |unit: &str| timers.iter().find(|t| t.unit == unit).cloned();
    let status = status.unwrap_or(Value::Null);
    Ok(json!({
        "unit": info.ok(),
        "timer": find(BACKUP_TIMER),
        "backups": files.map_err(agent_error)?,
        "r2": status.get("r2"),
        "restore_test": {
            "unit": test_info.ok(),
            "timer": find(RESTORE_TEST_TIMER),
            "last": status.get("restore_test"),
        },
    }))
}

pub async fn restore_test_schedule(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Json(req): Json<Schedule>,
) -> ApiResult<Json<Value>> {
    u.require_sudo()?;
    let on_calendar = if req.enabled {
        Some(req.on_calendar()?)
    } else {
        None
    };
    let res = st
        .agent
        .call::<Option<Timer>>(Method::TimerSchedule {
            timer: RESTORE_TEST_TIMER.into(),
            on_calendar: on_calendar.clone(),
        })
        .await;
    audit::user_action(
        &st,
        &u,
        &client,
        "backups.restore_test_schedule",
        RESTORE_TEST_TIMER,
        res.is_ok(),
        json!({ "on_calendar": on_calendar }),
    )
    .await;
    st.cache.invalidate("backups:");
    let timer = res.map_err(agent_error)?;
    settings::set(&st, RESTORE_TEST_KEY, &req)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(json!({ "timer": timer, "schedule": req })))
}

pub async fn timers(State(st): State<AppState>, _u: AuthUser) -> ApiResult<Json<Value>> {
    let v = st
        .cache
        .get_or("backups:timers", CACHE_TTL, || async {
            st.agent
                .call::<Value>(Method::Timers)
                .await
                .map_err(agent_error)
        })
        .await?;
    Ok(Json(v))
}

pub async fn unit(
    State(st): State<AppState>,
    _u: AuthUser,
    Path(unit): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        st.agent
            .call(Method::UnitInfo { unit })
            .await
            .map_err(agent_error)?,
    ))
}

#[derive(Deserialize)]
pub struct JournalQuery {
    lines: Option<u32>,
}

pub async fn unit_journal(
    State(st): State<AppState>,
    _u: AuthUser,
    Path(unit): Path<String>,
    Query(q): Query<JournalQuery>,
) -> ApiResult<Json<Vec<String>>> {
    let lines = q.lines.unwrap_or(300);
    Ok(Json(
        st.agent
            .call_timeout(Method::UnitJournal { unit, lines }, Duration::from_secs(30))
            .await
            .map_err(agent_error)?,
    ))
}

pub async fn run_unit(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Path(unit): Path<String>,
) -> ApiResult<Json<Value>> {
    let id = jobs::start(
        &st,
        jobs::Actor::user(&u, &client),
        "unit",
        &unit,
        Task::RunUnit { unit: unit.clone() },
    )
    .await?;
    Ok(Json(json!({ "job": id })))
}

// ---- databases ----------------------------------------------------------------

#[derive(Serialize)]
pub struct Database {
    name: String,
    size: i64,
    connections: i64,
}

#[derive(Serialize)]
pub struct PgServer {
    container: String,
    project: Option<String>,
    image: String,
    user: String,
    default_db: String,
    version: Option<String>,
    connections: Option<i64>,
    databases: Vec<Database>,
    error: Option<String>,
}

#[derive(Serialize)]
pub struct RedisServer {
    container: String,
    project: Option<String>,
    image: String,
    info: HashMap<String, String>,
    error: Option<String>,
}

fn kind_of(image: &str) -> Option<&'static str> {
    let base = image.rsplit('/').next().unwrap_or(image);
    if ["postgres", "postgis", "timescaledb", "pgvector"]
        .iter()
        .any(|p| base.starts_with(p))
    {
        Some("pg")
    } else if ["redis", "valkey", "keydb"]
        .iter()
        .any(|p| base.starts_with(p))
    {
        Some("redis")
    } else {
        None
    }
}

async fn pg_target(st: &AppState, container: &str) -> ApiResult<(String, String)> {
    let env = container_env(st, container)
        .await
        .map_err(|_| ApiError::NotFound)?;
    let user = env
        .get("POSTGRES_USER")
        .cloned()
        .unwrap_or_else(|| "postgres".into());
    let db = env
        .get("POSTGRES_DB")
        .cloned()
        .unwrap_or_else(|| user.clone());
    Ok((user, db))
}

async fn pg_server(
    st: &AppState,
    container: String,
    project: Option<String>,
    image: String,
) -> PgServer {
    let (user, default_db) = pg_target(st, &container)
        .await
        .unwrap_or_else(|_| ("postgres".into(), "postgres".into()));
    let mut s = PgServer {
        container,
        project,
        image,
        user,
        default_db,
        version: None,
        connections: None,
        databases: Vec::new(),
        error: None,
    };
    let out = exec(
        st,
        &s.container,
        &[
            "psql", "-U", &s.user, "-d", &s.default_db, "-AtX", "-F", "\t",
            "-c", "SELECT current_setting('server_version'), (SELECT count(*) FROM pg_stat_activity WHERE backend_type = 'client backend')",
            "-c", "SELECT datname, pg_database_size(datname), (SELECT count(*) FROM pg_stat_activity a WHERE a.datname = d.datname) FROM pg_database d WHERE NOT datistemplate ORDER BY 1",
        ],
        &[],
        Duration::from_secs(10),
    )
    .await;
    match out {
        Ok(o) if o.code == 0 => {
            for (i, line) in o.stdout.lines().enumerate() {
                let f: Vec<&str> = line.split('\t').collect();
                if i == 0 && f.len() == 2 {
                    s.version = Some(f[0].to_string());
                    s.connections = f[1].parse().ok();
                } else if f.len() == 3 {
                    s.databases.push(Database {
                        name: f[0].to_string(),
                        size: f[1].parse().unwrap_or(0),
                        connections: f[2].parse().unwrap_or(0),
                    });
                }
            }
        }
        Ok(o) => s.error = Some(o.stderr.trim().to_string()),
        Err(e) => s.error = Some(e.to_string()),
    }
    s
}

async fn redis_server(
    st: &AppState,
    container: String,
    project: Option<String>,
    image: String,
) -> RedisServer {
    let env = container_env(st, &container).await.unwrap_or_default();
    let auth = env
        .get("REDIS_PASSWORD")
        .or_else(|| env.get("VALKEY_PASSWORD"))
        .map(|p| format!("REDISCLI_AUTH={p}"));
    let envs: Vec<&str> = auth.iter().map(String::as_str).collect();
    let mut last_err = None;
    for cli in ["redis-cli", "valkey-cli", "keydb-cli"] {
        match exec(
            st,
            &container,
            &[cli, "INFO"],
            &envs,
            Duration::from_secs(5),
        )
        .await
        {
            Ok(o) if o.code == 0 && !o.stdout.contains("NOAUTH") => {
                let info = o
                    .stdout
                    .lines()
                    .filter_map(|l| l.trim().split_once(':'))
                    .filter(|(k, _)| {
                        matches!(
                            *k,
                            "redis_version"
                                | "valkey_version"
                                | "uptime_in_seconds"
                                | "connected_clients"
                                | "used_memory"
                                | "used_memory_peak"
                                | "maxmemory"
                                | "maxmemory_policy"
                                | "keyspace_hits"
                                | "keyspace_misses"
                                | "evicted_keys"
                                | "rdb_last_save_time"
                                | "rdb_last_bgsave_status"
                                | "aof_enabled"
                                | "role"
                        ) || k.starts_with("db")
                    })
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();
                return RedisServer {
                    container,
                    project,
                    image,
                    info,
                    error: None,
                };
            }
            Ok(o) => last_err = Some(format!("{}{}", o.stdout.trim(), o.stderr.trim())),
            Err(e) => last_err = Some(e.to_string()),
        }
    }
    RedisServer {
        container,
        project,
        image,
        info: HashMap::new(),
        error: last_err,
    }
}

/// One `docker exec` per database server, so the result is shared for 30 s; job ends,
/// dumps and restores invalidate `databases:`.
pub async fn databases(State(st): State<AppState>, _u: AuthUser) -> ApiResult<Json<Value>> {
    Ok(Json(
        st.cache
            .get_or("databases:list", CACHE_TTL, || databases_now(&st))
            .await?,
    ))
}

async fn databases_now(st: &AppState) -> ApiResult<Value> {
    let st = st.clone();
    let list = st
        .docker
        .list_containers(Some(ListContainersOptions::<String> {
            all: false,
            ..Default::default()
        }))
        .await?;
    let mut pg = Vec::new();
    let mut redis = Vec::new();
    for c in list {
        let image = c.image.clone().unwrap_or_default();
        let name = c
            .names
            .as_ref()
            .and_then(|n| n.first())
            .map(|n| n.trim_start_matches('/').to_string())
            .unwrap_or_default();
        let project = c
            .labels
            .as_ref()
            .and_then(|l| l.get("com.docker.compose.project").cloned());
        match kind_of(&image) {
            Some("pg") => pg.push(pg_server(&st, name, project, image)),
            Some("redis") => redis.push(redis_server(&st, name, project, image)),
            _ => {}
        }
    }
    let (mut pg, mut redis) = (
        futures::future::join_all(pg).await,
        futures::future::join_all(redis).await,
    );
    pg.sort_by(|a, b| a.container.cmp(&b.container));
    redis.sort_by(|a, b| a.container.cmp(&b.container));
    Ok(json!({ "postgres": pg, "redis": redis }))
}

#[derive(Deserialize)]
pub struct DumpReq {
    database: String,
}

pub async fn dump(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Path(container): Path<String>,
    Json(req): Json<DumpReq>,
) -> ApiResult<Json<Value>> {
    let (user, _) = pg_target(&st, &container).await?;
    let task = Task::PgDump {
        container: container.clone(),
        database: req.database,
        user,
        daily: false,
    };
    let id = jobs::start(&st, jobs::Actor::user(&u, &client), "pg", &container, task).await?;
    Ok(Json(json!({ "job": id })))
}

#[derive(Deserialize)]
pub struct RestoreReq {
    database: String,
    file: String,
    /// Must repeat the database name.
    confirm: String,
}

pub async fn restore(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Path(container): Path<String>,
    Json(req): Json<RestoreReq>,
) -> ApiResult<Json<Value>> {
    u.require_sudo()?;
    if req.confirm != req.database {
        return Err(ApiError::BadRequest(
            "введите имя базы для подтверждения".into(),
        ));
    }
    let (user, _) = pg_target(&st, &container).await?;
    let task = Task::PgRestore {
        container: container.clone(),
        database: req.database,
        user,
        file: req.file,
    };
    let id = jobs::start(&st, jobs::Actor::user(&u, &client), "pg", &container, task).await?;
    Ok(Json(json!({ "job": id })))
}

#[derive(Deserialize)]
pub struct QueryReq {
    database: String,
    sql: String,
}

const MAX_ROWS: usize = 1000;

pub async fn query(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Path(container): Path<String>,
    Json(req): Json<QueryReq>,
) -> ApiResult<Json<Value>> {
    u.require_sudo()?;
    let sql = req.sql.trim();
    if sql.is_empty() || sql.len() > 20_000 {
        return Err(ApiError::BadRequest(
            "пустой или слишком длинный запрос".into(),
        ));
    }
    // A leading backslash would make psql run a meta-command (`\!` is a shell).
    if sql.starts_with('\\') {
        return Err(ApiError::BadRequest(
            "команды psql (\\…) не поддерживаются".into(),
        ));
    }
    let (user, _) = pg_target(&st, &container).await?;
    let started = std::time::Instant::now();
    let out = exec(
        &st,
        &container,
        &[
            "psql",
            "-U",
            &user,
            "-d",
            &req.database,
            "-X",
            "-A",
            "-F",
            "\x1f",
            "-R",
            "\x1e",
            "-P",
            "footer=off",
            "-v",
            "ON_ERROR_STOP=1",
            "-c",
            sql,
        ],
        &["PGOPTIONS=-c default_transaction_read_only=on -c statement_timeout=10s"],
        Duration::from_secs(15),
    )
    .await
    .map_err(ApiError::Internal)?;
    audit::user_action(
        &st,
        &u,
        &client,
        "db.query",
        &format!("{container}/{}", req.database),
        out.code == 0,
        json!({ "sql": sql.chars().take(2000).collect::<String>() }),
    )
    .await;
    if out.code != 0 {
        return Ok(Json(json!({ "error": out.stderr.trim() })));
    }
    let body = out.stdout.trim_end_matches('\n');
    let mut records = body.split('\x1e').filter(|r| !r.is_empty());
    let columns: Vec<&str> = records
        .next()
        .map(|h| h.split('\x1f').collect())
        .unwrap_or_default();
    let mut rows = Vec::new();
    let mut truncated = false;
    for r in records {
        if rows.len() >= MAX_ROWS {
            truncated = true;
            break;
        }
        rows.push(r.split('\x1f').collect::<Vec<_>>());
    }
    Ok(Json(json!({
        "columns": columns,
        "rows": rows,
        "truncated": truncated,
        "ms": started.elapsed().as_millis() as u64,
        "notice": out.stderr.trim(),
    })))
}

#[cfg(test)]
mod tests {
    use super::{kind_of, Schedule};

    #[test]
    fn schedule_calendar() {
        let s = |days: &[&str], time: &str| Schedule {
            enabled: true,
            days: days.iter().map(|d| d.to_string()).collect(),
            time: time.into(),
        };
        assert_eq!(
            s(&["Thu", "Mon"], "5:07").on_calendar().unwrap(),
            "Mon,Thu *-*-* 05:07:00 UTC"
        );
        assert!(s(&[], "05:00").on_calendar().is_err());
        assert!(s(&["Mon", "Funday"], "05:00").on_calendar().is_err());
        assert!(s(&["Mon"], "24:00").on_calendar().is_err());
        assert!(s(&["Mon"], "05:00\nExecStart=x").on_calendar().is_err());
    }

    #[test]
    fn kinds() {
        assert_eq!(kind_of("postgres:18-alpine"), Some("pg"));
        assert_eq!(kind_of("pgvector/pgvector:pg17"), Some("pg"));
        assert_eq!(kind_of("redis:7"), Some("redis"));
        assert_eq!(kind_of("valkey/valkey:8"), Some("redis"));
        assert_eq!(kind_of("registry.example.org/team/api:latest"), None);
    }
}
