use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::response::Response;
use axum::Json;
use bollard::container::{InspectContainerOptions, LogOutput, LogsOptions};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth::{AuthUser, ClientInfo};
use crate::docker::{self, Container, DiskUsage, Project};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Serialize)]
pub struct ProjectsResp {
    projects: Vec<Project>,
    standalone: Vec<Container>,
}

pub async fn projects(State(st): State<AppState>, _u: AuthUser) -> ApiResult<Json<ProjectsResp>> {
    let (projects, standalone) = docker::projects(&st).await?;
    Ok(Json(ProjectsResp {
        projects,
        standalone,
    }))
}

pub async fn df(State(st): State<AppState>, _u: AuthUser) -> ApiResult<Json<DiskUsage>> {
    Ok(Json(docker::disk_usage(&st).await?))
}

fn valid_ref(id: &str) -> ApiResult<()> {
    let ok = !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if ok {
        Ok(())
    } else {
        Err(ApiError::BadRequest("invalid container reference".into()))
    }
}

const SECRET_HINTS: &[&str] = &[
    "PASS",
    "SECRET",
    "TOKEN",
    "KEY",
    "DSN",
    "DATABASE_URL",
    "PRIVATE",
    "CREDENTIAL",
];

/// Values of secret-looking variables are masked unless revealed in sudo mode.
fn mask_env(env: &[String], reveal: bool) -> Vec<Value> {
    env.iter()
        .map(|e| {
            let (k, v) = e.split_once('=').unwrap_or((e, ""));
            let secret = SECRET_HINTS.iter().any(|h| k.to_uppercase().contains(h));
            let hide = secret && !reveal;
            json!({ "key": k, "value": if hide { Value::Null } else { Value::String(v.into()) }, "masked": hide, "secret": secret })
        })
        .collect()
}

#[derive(Deserialize)]
pub struct InspectQuery {
    #[serde(default)]
    reveal: bool,
}

pub async fn inspect(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Path(id): Path<String>,
    Query(q): Query<InspectQuery>,
) -> ApiResult<Json<Value>> {
    valid_ref(&id)?;
    if q.reveal {
        u.require_sudo()?;
        crate::audit::user_action(
            &st,
            &u,
            &client,
            "container.reveal_env",
            &id,
            true,
            json!({}),
        )
        .await;
    }
    let c = st
        .docker
        .inspect_container(&id, None::<InspectContainerOptions>)
        .await?;
    let config = c.config.unwrap_or_default();
    let state = c.state.unwrap_or_default();
    let host = c.host_config.unwrap_or_default();
    Ok(Json(json!({
        "id": c.id,
        "name": c.name.map(|n| n.trim_start_matches('/').to_string()),
        "image": config.image,
        "created": c.created,
        "restart_count": c.restart_count,
        "state": {
            "status": state.status,
            "running": state.running,
            "started_at": state.started_at,
            "finished_at": state.finished_at,
            "exit_code": state.exit_code,
            "oom_killed": state.oom_killed,
            "health": state.health.as_ref().and_then(|h| h.status),
            "health_log": state.health.and_then(|h| h.log).unwrap_or_default().into_iter().rev().take(5)
                .map(|l| json!({ "exit_code": l.exit_code, "output": l.output, "end": l.end })).collect::<Vec<_>>(),
        },
        "labels": config.labels,
        "env": mask_env(&config.env.unwrap_or_default(), q.reveal),
        "cmd": config.cmd,
        "entrypoint": config.entrypoint,
        "restart_policy": host.restart_policy.and_then(|r| r.name),
        "memory_limit": host.memory,
        "nano_cpus": host.nano_cpus,
        "mounts": c.mounts.unwrap_or_default().into_iter()
            .map(|m| json!({ "type": m.typ, "source": m.source, "destination": m.destination, "rw": m.rw }))
            .collect::<Vec<_>>(),
        "networks": c.network_settings.and_then(|n| n.networks).map(|n| n.into_keys().collect::<Vec<_>>()),
    })))
}

#[derive(Deserialize)]
pub struct LogsQuery {
    #[serde(default = "default_tail")]
    tail: u32,
    #[serde(default = "default_true")]
    follow: bool,
}
fn default_tail() -> u32 {
    500
}
fn default_true() -> bool {
    true
}

pub async fn logs(
    State(st): State<AppState>,
    _u: AuthUser,
    Path(id): Path<String>,
    Query(q): Query<LogsQuery>,
    ws: WebSocketUpgrade,
) -> ApiResult<Response> {
    valid_ref(&id)?;
    Ok(ws.on_upgrade(move |socket| stream_logs(st, id, q, socket)))
}

/// Each WS text frame is a batch of lines: `[{"s":"out"|"err","t":"<line with timestamp>"}]`.
async fn stream_logs(st: AppState, id: String, q: LogsQuery, mut socket: WebSocket) {
    let opts = LogsOptions::<String> {
        follow: q.follow,
        stdout: true,
        stderr: true,
        timestamps: true,
        tail: q.tail.min(10_000).to_string(),
        ..Default::default()
    };
    let mut logs = st.docker.logs(&id, Some(opts)).ready_chunks(256);
    loop {
        tokio::select! {
            chunk = logs.next() => {
                let Some(chunk) = chunk else { break };
                let mut batch = Vec::with_capacity(chunk.len());
                for item in chunk {
                    match item {
                        Ok(LogOutput::StdErr { message }) => batch.push(json!({ "s": "err", "t": String::from_utf8_lossy(&message) })),
                        Ok(LogOutput::StdOut { message } | LogOutput::Console { message }) =>
                            batch.push(json!({ "s": "out", "t": String::from_utf8_lossy(&message) })),
                        Ok(_) => {}
                        Err(e) => batch.push(json!({ "s": "sys", "t": format!("log stream error: {e}") })),
                    }
                }
                if socket.send(Message::Text(Value::Array(batch).to_string().into())).await.is_err() {
                    return;
                }
            }
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                _ => {}
            },
        }
    }
    let _ = socket
        .send(Message::Text(
            json!([{ "s": "sys", "t": "stream ended" }])
                .to_string()
                .into(),
        ))
        .await;
}
