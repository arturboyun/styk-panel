use axum::extract::{Query, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::auth::AuthUser;
use crate::error::ApiResult;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct AuditQuery {
    before: Option<i64>,
    action: Option<String>,
    #[serde(default = "default_limit")]
    limit: i64,
}
fn default_limit() -> i64 {
    100
}

#[derive(Serialize, sqlx::FromRow)]
pub struct AuditRow {
    id: i64,
    ts: DateTime<Utc>,
    username: Option<String>,
    action: String,
    target: Option<String>,
    ip: Option<String>,
    ok: bool,
    details: Value,
}

pub async fn list(
    State(st): State<AppState>,
    _u: AuthUser,
    Query(q): Query<AuditQuery>,
) -> ApiResult<Json<Vec<AuditRow>>> {
    let rows = sqlx::query_as::<_, AuditRow>(
        "SELECT id, ts, username, action, target, ip, ok, details FROM audit_log
          WHERE ($1::bigint IS NULL OR id < $1) AND ($2::text IS NULL OR action LIKE $2 || '%')
          ORDER BY id DESC LIMIT $3",
    )
    .bind(q.before)
    .bind(q.action)
    .bind(q.limit.clamp(1, 500))
    .fetch_all(&st.db)
    .await?;
    Ok(Json(rows))
}
