//! Jobs: agent tasks tracked in Postgres. The panel polls the agent for output, keeps the
//! running tail in memory for the UI and appends it to `jobs.log`; after a panel restart
//! unfinished jobs are picked up again from `log_offset`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;
use styk_proto::{Method, Task, TaskOutput, TaskStarted};

use crate::audit;
use crate::auth::{AuthUser, ClientInfo};
use crate::error::{agent_error, ApiError, ApiResult};
use crate::state::AppState;

const POLL: Duration = Duration::from_millis(700);
const FLUSH: Duration = Duration::from_secs(3);
/// How long the agent may stay unreachable (it restarts on upgrades) before a job is lost.
const AGENT_GRACE: Duration = Duration::from_secs(180);

#[derive(Default)]
pub struct Jobs {
    live: Mutex<HashMap<i64, Arc<Mutex<Vec<String>>>>>,
}

impl Jobs {
    /// Lines of a running job from `offset`, or None when it is not running here.
    pub fn tail(&self, id: i64, offset: usize) -> Option<(Vec<String>, usize)> {
        let live = self.live.lock().unwrap();
        let buf = live.get(&id)?.lock().unwrap();
        let from = offset.min(buf.len());
        Some((buf[from..].to_vec(), buf.len()))
    }
}

/// Who started a job: a signed-in user or a deploy webhook.
pub struct Actor<'a> {
    pub user_id: Option<i64>,
    pub username: &'a str,
    pub ip: &'a str,
}

impl<'a> Actor<'a> {
    pub fn user(u: &'a AuthUser, c: &'a ClientInfo) -> Self {
        Actor {
            user_id: Some(u.user_id),
            username: &u.username,
            ip: &c.ip,
        }
    }
}

pub async fn start(
    state: &AppState,
    actor: Actor<'_>,
    kind: &str,
    target: &str,
    task: Task,
) -> ApiResult<i64> {
    let busy: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM jobs WHERE kind = $1 AND target = $2 AND status = 'running' LIMIT 1",
    )
    .bind(kind)
    .bind(target)
    .fetch_optional(&state.db)
    .await?;
    if let Some(id) = busy {
        return Err(ApiError::BadRequest(format!(
            "уже выполняется (задача #{id})"
        )));
    }
    let details = serde_json::to_value(&task).unwrap_or_default();
    let started: Result<TaskStarted, _> = state.agent.call(Method::TaskStart { task }).await;
    audit::record(
        state,
        audit::Entry {
            user_id: actor.user_id,
            username: Some(actor.username),
            action: "job.start",
            target: Some(&format!("{kind}:{target}")),
            ip: actor.ip,
            ok: started.is_ok(),
            details: json!({ "task": details }),
        },
    )
    .await;
    let started = started.map_err(agent_error)?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO jobs (kind, target, user_id, username, agent_task) VALUES ($1,$2,$3,$4,$5) RETURNING id",
    )
    .bind(kind)
    .bind(target)
    .bind(actor.user_id)
    .bind(actor.username)
    .bind(started.id as i64)
    .fetch_one(&state.db)
    .await?;
    spawn_follow(state.clone(), id, started.id, 0, Vec::new());
    invalidate_views(state);
    Ok(id)
}

/// Deploys, prunes, unit runs and backups change what these cached views show.
fn invalidate_views(state: &AppState) {
    for prefix in ["docker:", "backups:", "databases:"] {
        state.cache.invalidate(prefix);
    }
}

/// Re-attach to jobs that were running when the panel stopped.
pub async fn resume(state: &AppState) {
    let rows: Vec<(i64, Option<i64>, i32, String)> = match sqlx::query_as(
        "SELECT id, agent_task, log_offset, log FROM jobs WHERE status = 'running'",
    )
    .fetch_all(&state.db)
    .await
    {
        Ok(r) => r,
        Err(e) => return tracing::error!("resume jobs: {e}"),
    };
    for (id, task, offset, log) in rows {
        match task {
            Some(t) => {
                let lines = log.lines().map(str::to_string).collect();
                spawn_follow(state.clone(), id, t as u64, offset as usize, lines);
            }
            None => finish(state, id, "lost", None, "").await,
        }
    }
}

fn spawn_follow(state: AppState, id: i64, task: u64, offset: usize, lines: Vec<String>) {
    let buf = Arc::new(Mutex::new(lines));
    state.jobs.live.lock().unwrap().insert(id, buf.clone());
    tokio::spawn(async move {
        follow(&state, id, task, offset, &buf).await;
        state.jobs.live.lock().unwrap().remove(&id);
    });
}

async fn follow(state: &AppState, id: i64, task: u64, mut offset: usize, buf: &Mutex<Vec<String>>) {
    let mut pending = String::new();
    let mut last_flush = Instant::now();
    let mut last_ok = Instant::now();
    loop {
        tokio::time::sleep(POLL).await;
        let out: TaskOutput = match state
            .agent
            .call(Method::TaskPoll { id: task, offset })
            .await
        {
            Ok(o) => o,
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("unknown task") || last_ok.elapsed() > AGENT_GRACE {
                    pending.push_str(&format!("✗ потеряна связь с задачей агента: {msg}\n"));
                    return finish(state, id, "lost", None, &pending).await;
                }
                continue;
            }
        };
        last_ok = Instant::now();
        offset = out.next;
        if !out.lines.is_empty() {
            let mut b = buf.lock().unwrap();
            for l in &out.lines {
                pending.push_str(l);
                pending.push('\n');
            }
            b.extend(out.lines);
        }
        if out.done {
            let status = if out.exit_code == Some(0) {
                "ok"
            } else {
                "failed"
            };
            return finish(state, id, status, out.exit_code, &pending).await;
        }
        if !pending.is_empty() && last_flush.elapsed() > FLUSH {
            let res = sqlx::query("UPDATE jobs SET log = log || $2, log_offset = $3 WHERE id = $1")
                .bind(id)
                .bind(&pending)
                .bind(offset as i32)
                .execute(&state.db)
                .await;
            if res.is_ok() {
                pending.clear();
                last_flush = Instant::now();
            }
        }
    }
}

async fn finish(state: &AppState, id: i64, status: &str, code: Option<i32>, pending: &str) {
    let res = sqlx::query(
        "UPDATE jobs SET log = log || $2, status = $3, exit_code = $4, finished_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(pending)
    .bind(status)
    .bind(code)
    .execute(&state.db)
    .await;
    if let Err(e) = res {
        tracing::error!(id, "finish job: {e}");
    }
    tracing::info!(id, status, "job finished");
    invalidate_views(state);
    if status != "ok" {
        let row: Option<(String, String)> =
            sqlx::query_as("SELECT kind, target FROM jobs WHERE id = $1")
                .bind(id)
                .fetch_optional(&state.db)
                .await
                .ok()
                .flatten();
        if let Some((kind, target)) = row {
            let msg = format!(
                "Задача #{id} {kind}:{target} — {status}{}",
                code.map(|c| format!(", код {c}")).unwrap_or_default()
            );
            crate::alerts::fire_event(
                state,
                "job_failed",
                &format!("{kind}:{target}"),
                "warning",
                &msg,
            )
            .await;
        }
    }
}
