use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Serialize, sqlx::FromRow)]
pub struct JobRow {
    id: i64,
    kind: String,
    target: String,
    status: String,
    username: Option<String>,
    exit_code: Option<i32>,
    started_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
pub struct ListQuery {
    /// One kind or several, comma-separated.
    kind: Option<String>,
    target: Option<String>,
    limit: Option<i64>,
}

pub async fn list(
    _u: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<JobRow>>> {
    let rows = sqlx::query_as::<_, JobRow>(
        "SELECT id, kind, target, status, username, exit_code, started_at, finished_at FROM jobs
          WHERE ($1::text[] IS NULL OR kind = ANY($1)) AND ($2::text IS NULL OR target = $2)
          ORDER BY id DESC LIMIT $3",
    )
    .bind(
        q.kind
            .map(|k| k.split(',').map(str::to_string).collect::<Vec<_>>()),
    )
    .bind(q.target)
    .bind(q.limit.unwrap_or(30).clamp(1, 200))
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
pub struct TailQuery {
    #[serde(default)]
    offset: usize,
}

#[derive(Serialize)]
pub struct JobTail {
    #[serde(flatten)]
    job: JobRow,
    lines: Vec<String>,
    next: usize,
}

pub async fn get(
    _u: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<TailQuery>,
) -> ApiResult<Json<JobTail>> {
    let (job, mut log): (JobRow, String) = {
        #[derive(sqlx::FromRow)]
        struct Row {
            #[sqlx(flatten)]
            job: JobRow,
            log: String,
        }
        // A running job's output comes from the in-memory buffer, so skip the stored log.
        let r = sqlx::query_as::<_, Row>(
            "SELECT id, kind, target, status, username, exit_code, started_at, finished_at,
                    CASE WHEN status = 'running' THEN '' ELSE log END AS log
               FROM jobs WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)?;
        (r.job, r.log)
    };
    // Running here: the in-memory buffer is ahead of the database.
    let tail = (job.status == "running")
        .then(|| state.jobs.tail(id, q.offset))
        .flatten();
    if tail.is_none() && job.status == "running" {
        // Not in memory (just started or just finished): read what is stored.
        log = sqlx::query_scalar("SELECT log FROM jobs WHERE id = $1")
            .bind(id)
            .fetch_one(&state.db)
            .await?;
    }
    let (lines, next) = match tail {
        Some(t) => t,
        None => {
            let all: Vec<&str> = log.lines().collect();
            let from = q.offset.min(all.len());
            (
                all[from..].iter().map(|s| s.to_string()).collect(),
                all.len(),
            )
        }
    };
    Ok(Json(JobTail { job, lines, next }))
}
