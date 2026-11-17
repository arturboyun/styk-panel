use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::alerts::{self, Prefs};
use crate::audit;
use crate::auth::{AuthUser, ClientInfo};
use crate::error::{ApiError, ApiResult};
use crate::notify::{self, LastStart};
use crate::settings;
use crate::state::AppState;

#[derive(Serialize, sqlx::FromRow)]
pub struct Event {
    id: i64,
    rule: String,
    subject: String,
    severity: String,
    status: String,
    message: String,
    started_at: DateTime<Utc>,
    resolved_at: Option<DateTime<Utc>>,
    acked_by: Option<String>,
    acked_at: Option<DateTime<Utc>>,
    muted_until: Option<DateTime<Utc>>,
    last_notified_at: Option<DateTime<Utc>>,
}

const EVENT_COLS: &str = "id, rule, subject, severity, status, message, started_at, resolved_at, acked_by, acked_at, muted_until, last_notified_at";

pub async fn overview(State(st): State<AppState>, _u: AuthUser) -> ApiResult<Json<Value>> {
    let active: Vec<Event> = sqlx::query_as(&format!(
        "SELECT {EVENT_COLS} FROM alert_events WHERE status = 'firing'
          ORDER BY CASE severity WHEN 'critical' THEN 0 WHEN 'warning' THEN 1 ELSE 2 END, started_at DESC"
    ))
    .fetch_all(&st.db)
    .await?;
    let history: Vec<Event> = sqlx::query_as(&format!(
        "SELECT {EVENT_COLS} FROM alert_events WHERE status = 'resolved' ORDER BY started_at DESC LIMIT 150"
    ))
    .fetch_all(&st.db)
    .await?;
    let overrides: Vec<(String, bool, Value, Option<DateTime<Utc>>)> =
        sqlx::query_as("SELECT key, enabled, params, muted_until FROM alert_rules")
            .fetch_all(&st.db)
            .await?;
    let rules: Vec<Value> = alerts::rules()
        .into_iter()
        .map(|r| {
            let o = overrides.iter().find(|o| o.0 == r.key);
            let mut params = r.defaults.clone();
            if let (Some(p), Some(o)) = (params.as_object_mut(), o.and_then(|o| o.2.as_object())) {
                for (k, v) in o {
                    p.insert(k.clone(), v.clone());
                }
            }
            json!({
                "def": r,
                "enabled": o.is_none_or(|o| o.1),
                "params": params,
                "muted_until": o.and_then(|o| o.3),
            })
        })
        .collect();
    let last_start: Option<LastStart> = settings::get(&st, notify::LAST_START_KEY).await;
    Ok(Json(json!({
        "active": active,
        "history": history,
        "rules": rules,
        "prefs": alerts::prefs(&st).await,
        "telegram": {
            "configured": st.cfg.telegram_bot_token.is_some(),
            "bot_username": st.cfg.telegram_bot_username,
            "chat_id": notify::chat_id(&st).await,
            "last_start": last_start,
        },
        "heartbeat": st.cfg.heartbeat_url.is_some(),
    })))
}

#[derive(Deserialize)]
pub struct RuleReq {
    enabled: bool,
    params: Value,
}

pub async fn update_rule(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Path(key): Path<String>,
    Json(req): Json<RuleReq>,
) -> ApiResult<Json<Value>> {
    let def = alerts::rules()
        .into_iter()
        .find(|r| r.key == key)
        .ok_or(ApiError::NotFound)?;
    // Keep only known parameters with the default's JSON type.
    let mut params = serde_json::Map::new();
    if let (Some(d), Some(p)) = (def.defaults.as_object(), req.params.as_object()) {
        for (k, v) in p {
            let same = d.get(k).is_some_and(|dv| {
                (dv.is_number() && v.is_number())
                    || (dv.is_array() && v.is_array())
                    || (dv.is_boolean() && v.is_boolean())
            });
            if same {
                params.insert(k.clone(), v.clone());
            }
        }
    }
    sqlx::query(
        "INSERT INTO alert_rules (key, enabled, params) VALUES ($1, $2, $3)
         ON CONFLICT (key) DO UPDATE SET enabled = EXCLUDED.enabled, params = EXCLUDED.params",
    )
    .bind(&key)
    .bind(req.enabled)
    .bind(Value::Object(params.clone()))
    .execute(&st.db)
    .await?;
    audit::user_action(
        &st,
        &u,
        &client,
        "alert.rule",
        &key,
        true,
        json!({ "enabled": req.enabled, "params": params }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct MuteReq {
    /// 0 unmutes.
    hours: i32,
}

pub async fn mute_rule(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Path(key): Path<String>,
    Json(req): Json<MuteReq>,
) -> ApiResult<Json<Value>> {
    alerts::rules()
        .into_iter()
        .find(|r| r.key == key)
        .ok_or(ApiError::NotFound)?;
    sqlx::query(
        "INSERT INTO alert_rules (key, muted_until) VALUES ($1, CASE WHEN $2 > 0 THEN now() + make_interval(hours => $2) END)
         ON CONFLICT (key) DO UPDATE SET muted_until = EXCLUDED.muted_until",
    )
    .bind(&key)
    .bind(req.hours.clamp(0, 24 * 30))
    .execute(&st.db)
    .await?;
    audit::user_action(
        &st,
        &u,
        &client,
        "alert.mute_rule",
        &key,
        true,
        json!({ "hours": req.hours }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

pub async fn ack(
    State(st): State<AppState>,
    u: AuthUser,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    sqlx::query("UPDATE alert_events SET acked_by = $2, acked_at = now() WHERE id = $1")
        .bind(id)
        .bind(&u.username)
        .execute(&st.db)
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn mute_event(
    State(st): State<AppState>,
    u: AuthUser,
    Path(id): Path<i64>,
    Json(req): Json<MuteReq>,
) -> ApiResult<Json<Value>> {
    sqlx::query(
        "UPDATE alert_events SET muted_until = CASE WHEN $2 > 0 THEN now() + make_interval(hours => $2) END,
                acked_by = COALESCE(acked_by, $3), acked_at = COALESCE(acked_at, now()) WHERE id = $1",
    )
    .bind(id)
    .bind(req.hours.clamp(0, 24 * 30))
    .bind(&u.username)
    .execute(&st.db)
    .await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct SettingsReq {
    prefs: Prefs,
    chat_id: Option<i64>,
}

pub async fn save_settings(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Json(req): Json<SettingsReq>,
) -> ApiResult<Json<Value>> {
    let mut p = req.prefs;
    p.quiet_from %= 24;
    p.quiet_to %= 24;
    p.summary_hour %= 24;
    p.repeat_hours = p.repeat_hours.clamp(1, 168);
    p.tz_offset_min = p.tz_offset_min.clamp(-720, 840);
    settings::set(&st, alerts::PREFS_KEY, &p).await?;
    match req.chat_id {
        Some(c) => settings::set(&st, notify::CHAT_KEY, &c).await?,
        None => {
            sqlx::query("DELETE FROM settings WHERE key = $1")
                .bind(notify::CHAT_KEY)
                .execute(&st.db)
                .await?;
        }
    }
    audit::user_action(
        &st,
        &u,
        &client,
        "alert.settings",
        "alerts",
        true,
        json!({ "chat_id": req.chat_id }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

pub async fn telegram_test(State(st): State<AppState>, _u: AuthUser) -> ApiResult<Json<Value>> {
    let Some(chat) = notify::chat_id(&st).await else {
        return Err(ApiError::BadRequest("сначала укажите чат".into()));
    };
    notify::send_to(
        &st,
        chat,
        "🔔 <b>Styk Panel</b>: тестовое уведомление. Алерты будут приходить сюда.",
        json!([]),
    )
    .await
    .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn summary_now(State(st): State<AppState>, _u: AuthUser) -> ApiResult<Json<Value>> {
    let html = alerts::summary(&st).await;
    let sent = notify::send(&st, &html, json!([]))
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok(Json(json!({ "sent": sent, "html": html })))
}

// ---- uptime -----------------------------------------------------------------

#[derive(Serialize, sqlx::FromRow)]
pub struct Check {
    id: i64,
    url: String,
    name: Option<String>,
    source: String,
    enabled: bool,
    up: Option<bool>,
    status: Option<i32>,
    latency_ms: Option<i32>,
    error: Option<String>,
    last_ts: Option<DateTime<Utc>>,
    uptime_24h: Option<f64>,
    uptime_7d: Option<f64>,
    avg_latency_24h: Option<f64>,
    recent: Option<Vec<bool>>,
}

pub async fn uptime(State(st): State<AppState>, _u: AuthUser) -> ApiResult<Json<Vec<Check>>> {
    let rows = sqlx::query_as::<_, Check>(
        "SELECT c.id, c.url, c.name, c.source, c.enabled,
                l.up, l.status, l.latency_ms, l.error, l.ts AS last_ts,
                (SELECT avg(up::int)::float8 * 100 FROM uptime_results WHERE check_id = c.id AND ts > now() - interval '24 hours') AS uptime_24h,
                (SELECT avg(up::int)::float8 * 100 FROM uptime_results WHERE check_id = c.id AND ts > now() - interval '7 days') AS uptime_7d,
                (SELECT avg(latency_ms)::float8 FROM uptime_results WHERE check_id = c.id AND up AND ts > now() - interval '24 hours') AS avg_latency_24h,
                (SELECT array_agg(up ORDER BY ts) FROM (SELECT up, ts FROM uptime_results WHERE check_id = c.id ORDER BY ts DESC LIMIT 60) x) AS recent
           FROM uptime_checks c
           LEFT JOIN LATERAL (SELECT up, status, latency_ms, error, ts FROM uptime_results WHERE check_id = c.id ORDER BY ts DESC LIMIT 1) l ON true
          ORDER BY c.url",
    )
    .fetch_all(&st.db)
    .await?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    hours: Option<i32>,
}

pub async fn uptime_history(
    State(st): State<AppState>,
    _u: AuthUser,
    Path(id): Path<i64>,
    Query(q): Query<HistoryQuery>,
) -> ApiResult<Json<Value>> {
    let rows: Vec<(
        DateTime<Utc>,
        bool,
        Option<i32>,
        Option<i32>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT ts, up, status, latency_ms, error FROM uptime_results
          WHERE check_id = $1 AND ts > now() - make_interval(hours => $2) ORDER BY ts",
    )
    .bind(id)
    .bind(q.hours.unwrap_or(24).clamp(1, 24 * 30))
    .fetch_all(&st.db)
    .await?;
    Ok(Json(json!(rows
        .into_iter()
        .map(|(ts, up, status, latency, error)| json!({ "ts": ts.timestamp(), "up": up, "status": status, "latency_ms": latency, "error": error }))
        .collect::<Vec<_>>())))
}

#[derive(Deserialize)]
pub struct NewCheck {
    url: String,
    name: Option<String>,
}

pub async fn add_check(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Json(req): Json<NewCheck>,
) -> ApiResult<Json<Value>> {
    let url = req.url.trim().to_string();
    let parsed =
        reqwest::Url::parse(&url).map_err(|_| ApiError::BadRequest("некорректный URL".into()))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(ApiError::BadRequest("нужен http(s) URL".into()));
    }
    sqlx::query("INSERT INTO uptime_checks (url, name, source) VALUES ($1, $2, 'manual') ON CONFLICT (url) DO NOTHING")
        .bind(&url)
        .bind(req.name.filter(|n| !n.trim().is_empty()))
        .execute(&st.db)
        .await?;
    audit::user_action(&st, &u, &client, "uptime.add", &url, true, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct ToggleReq {
    enabled: bool,
}

pub async fn toggle_check(
    State(st): State<AppState>,
    _u: AuthUser,
    Path(id): Path<i64>,
    Json(req): Json<ToggleReq>,
) -> ApiResult<Json<Value>> {
    sqlx::query("UPDATE uptime_checks SET enabled = $2 WHERE id = $1")
        .bind(id)
        .bind(req.enabled)
        .execute(&st.db)
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn delete_check(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let url: Option<String> = sqlx::query_scalar(
        "DELETE FROM uptime_checks WHERE id = $1 AND source = 'manual' RETURNING url",
    )
    .bind(id)
    .fetch_optional(&st.db)
    .await?;
    let url = url.ok_or_else(|| {
        ApiError::BadRequest("проверки сайтов из Caddyfile можно только выключить".into())
    })?;
    audit::user_action(&st, &u, &client, "uptime.delete", &url, true, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}
