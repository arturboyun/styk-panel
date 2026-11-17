//! Docker mutations: compose project actions (as agent jobs), container lifecycle,
//! exec terminal, prune, deploy webhooks.

use std::time::Duration;

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::Json;
use bollard::container::{KillContainerOptions, RestartContainerOptions, StopContainerOptions};
use bollard::exec::{CreateExecOptions, ResizeExecOptions, StartExecResults};
use futures::StreamExt;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;
use styk_proto::{ComposeAction, PruneImages, Task};
use tokio::io::AsyncWriteExt;

use crate::audit;
use crate::auth::{AuthUser, ClientInfo};
use crate::crypto::random_token;
use crate::docker;
use crate::error::{ApiError, ApiResult};
use crate::images;
use crate::jobs::{self, Actor};
use crate::state::AppState;

fn is_proxy(service: &str) -> bool {
    service.contains("caddy")
}

async fn find_project(st: &AppState, name: &str) -> ApiResult<docker::Project> {
    let (projects, _) = docker::projects(st).await?;
    projects
        .into_iter()
        .find(|p| p.name == name)
        .ok_or(ApiError::NotFound)
}

fn compose_task(
    p: &docker::Project,
    action: ComposeAction,
    services: Vec<String>,
) -> ApiResult<Task> {
    let working_dir = p
        .working_dir
        .clone()
        .ok_or_else(|| ApiError::BadRequest("у проекта нет working_dir в labels".into()))?;
    Ok(Task::Compose {
        project: p.name.clone(),
        working_dir,
        config_files: p.config_files.clone(),
        action,
        services,
    })
}

#[derive(Deserialize)]
pub struct ProjectActionReq {
    action: ComposeAction,
    #[serde(default)]
    services: Vec<String>,
}

pub async fn project_action(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Path(name): Path<String>,
    Json(req): Json<ProjectActionReq>,
) -> ApiResult<Json<Value>> {
    u.require_sudo()?;
    let p = find_project(&st, &name).await?;
    if p.protected {
        if matches!(req.action, ComposeAction::Stop | ComposeAction::Down) {
            return Err(ApiError::Forbidden(
                "protected",
                format!("{name} защищён: остановка запрещена"),
            ));
        }
        if req.services.is_empty() || req.services.iter().any(|s| is_proxy(s)) {
            return Err(ApiError::Forbidden(
                "protected",
                format!("{name} защищён: выберите отдельные сервисы (кроме Caddy)"),
            ));
        }
    }
    let task = compose_task(&p, req.action, req.services)?;
    let id = jobs::start(&st, Actor::user(&u, &client), "compose", &name, task).await?;
    Ok(Json(json!({ "job": id })))
}

#[derive(Deserialize)]
pub struct ContainerActionReq {
    /// start | stop | restart | kill
    action: String,
}

pub async fn container_action(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Path(id): Path<String>,
    Json(req): Json<ContainerActionReq>,
) -> ApiResult<Json<Value>> {
    let info = st.docker.inspect_container(&id, None).await?;
    let labels = info.config.and_then(|c| c.labels).unwrap_or_default();
    let project = labels
        .get("com.docker.compose.project")
        .cloned()
        .unwrap_or_default();
    let service = labels
        .get("com.docker.compose.service")
        .cloned()
        .unwrap_or_default();
    let name = info
        .name
        .unwrap_or_default()
        .trim_start_matches('/')
        .to_string();
    let protected = st.cfg.protected_projects.contains(&project);
    match req.action.as_str() {
        "stop" | "kill" => {
            u.require_sudo()?;
            if protected {
                return Err(ApiError::Forbidden(
                    "protected",
                    format!("{project} защищён: остановка запрещена"),
                ));
            }
        }
        "restart" if protected && is_proxy(&service) => {
            return Err(ApiError::Forbidden(
                "protected",
                "Caddy только перезагружает конфиг, не перезапускается".into(),
            ));
        }
        "start" | "restart" => {}
        _ => return Err(ApiError::BadRequest("unknown action".into())),
    }
    let res = match req.action.as_str() {
        "start" => st.docker.start_container::<String>(&id, None).await,
        "stop" => {
            st.docker
                .stop_container(&id, Some(StopContainerOptions { t: 20 }))
                .await
        }
        "restart" => {
            st.docker
                .restart_container(&id, Some(RestartContainerOptions { t: 20 }))
                .await
        }
        _ => {
            st.docker
                .kill_container(&id, Some(KillContainerOptions { signal: "SIGKILL" }))
                .await
        }
    };
    audit::user_action(
        &st,
        &u,
        &client,
        &format!("container.{}", req.action),
        &name,
        res.is_ok(),
        json!({}),
    )
    .await;
    res?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct PruneReq {
    build_cache: bool,
    images: PruneImages,
}

pub async fn prune(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Json(req): Json<PruneReq>,
) -> ApiResult<Json<Value>> {
    u.require_sudo()?;
    let task = Task::DockerPrune {
        build_cache: req.build_cache,
        images: req.images,
    };
    let id = jobs::start(&st, Actor::user(&u, &client), "docker-prune", "host", task).await?;
    Ok(Json(json!({ "job": id })))
}

// ---- image retention ----------------------------------------------------------

/// Policy, last run and what the policy would remove right now.
#[derive(Deserialize)]
pub struct RetentionPreview {
    keep: Option<usize>,
    /// Comma-separated.
    protected: Option<String>,
}

pub async fn retention(
    State(st): State<AppState>,
    _u: AuthUser,
    Query(q): Query<RetentionPreview>,
) -> ApiResult<Json<Value>> {
    let policy = images::policy(&st).await;
    let mut preview = policy.clone();
    if let Some(k) = q.keep {
        preview.keep = k.clamp(1, 50);
    }
    if let Some(p) = q.protected {
        preview.protected_tags = p
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
    }
    let plan = images::current_plan(&st, &preview).await?;
    Ok(Json(json!({
        "policy": policy,
        "plan": plan,
        "last": images::last_run(&st).await,
        "schedule": "04:30 UTC",
    })))
}

#[derive(Deserialize)]
pub struct RetentionReq {
    enabled: bool,
    keep: usize,
    protected_tags: Vec<String>,
}

fn retention_policy(req: RetentionReq) -> ApiResult<images::Policy> {
    if !(1..=50).contains(&req.keep) {
        return Err(ApiError::BadRequest("N должно быть от 1 до 50".into()));
    }
    let protected_tags = req
        .protected_tags
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    Ok(images::Policy {
        enabled: req.enabled,
        keep: req.keep,
        protected_tags,
    })
}

pub async fn save_retention(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Json(req): Json<RetentionReq>,
) -> ApiResult<Json<Value>> {
    u.require_sudo()?;
    let policy = retention_policy(req)?;
    images::save_policy(&st, &policy).await?;
    audit::user_action(
        &st,
        &u,
        &client,
        "images.retention.policy",
        "host",
        true,
        json!(policy),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

/// Applies the posted policy now (without saving it), so the preview and the run match.
pub async fn run_retention(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Json(req): Json<RetentionReq>,
) -> ApiResult<Json<images::LastRun>> {
    u.require_sudo()?;
    let policy = retention_policy(req)?;
    Ok(Json(
        images::run(&st, &policy, "manual", Some((&u, &client))).await?,
    ))
}

// ---- exec terminal -----------------------------------------------------------

const EXEC_IDLE: Duration = Duration::from_secs(15 * 60);

#[derive(Deserialize)]
pub struct ExecQuery {
    #[serde(default = "default_cols")]
    cols: u16,
    #[serde(default = "default_rows")]
    rows: u16,
}

fn default_cols() -> u16 {
    120
}
fn default_rows() -> u16 {
    32
}

pub async fn exec(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Path(id): Path<String>,
    Query(q): Query<ExecQuery>,
    ws: WebSocketUpgrade,
) -> ApiResult<Response> {
    u.require_sudo()?;
    let info = st.docker.inspect_container(&id, None).await?;
    let name = info
        .name
        .unwrap_or_default()
        .trim_start_matches('/')
        .to_string();
    Ok(ws.on_upgrade(move |socket| async move {
        audit::user_action(
            &st,
            &u,
            &client,
            "container.exec",
            &name,
            true,
            json!({ "event": "open" }),
        )
        .await;
        let started = std::time::Instant::now();
        if let Err(e) = run_exec(&st, &id, q, socket).await {
            tracing::debug!("exec session: {e:#}");
        }
        let secs = started.elapsed().as_secs();
        audit::user_action(
            &st,
            &u,
            &client,
            "container.exec",
            &name,
            true,
            json!({ "event": "close", "seconds": secs }),
        )
        .await;
    }))
}

/// Client → server: text frames `{"t":"i","d":"<input>"}` or `{"t":"r","c":cols,"r":rows}`.
/// Server → client: binary frames with terminal output, a final text frame on exit.
async fn run_exec(
    st: &AppState,
    id: &str,
    q: ExecQuery,
    mut socket: WebSocket,
) -> anyhow::Result<()> {
    let exec = st
        .docker
        .create_exec(
            id,
            CreateExecOptions {
                cmd: Some(vec![
                    "sh".to_string(),
                    "-c".into(),
                    "if command -v bash >/dev/null 2>&1; then exec bash -l; else exec sh -l; fi"
                        .into(),
                ]),
                env: Some(vec!["TERM=xterm-256color".to_string()]),
                attach_stdin: Some(true),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                tty: Some(true),
                ..Default::default()
            },
        )
        .await?;
    let StartExecResults::Attached {
        mut output,
        mut input,
    } = st.docker.start_exec(&exec.id, None).await?
    else {
        anyhow::bail!("exec detached");
    };
    let _ = st
        .docker
        .resize_exec(
            &exec.id,
            ResizeExecOptions {
                width: q.cols,
                height: q.rows,
            },
        )
        .await;
    loop {
        tokio::select! {
            out = output.next() => match out {
                Some(Ok(chunk)) => socket.send(Message::Binary(chunk.into_bytes())).await?,
                _ => break,
            },
            msg = tokio::time::timeout(EXEC_IDLE, socket.recv()) => {
                let Ok(Some(Ok(msg))) = msg else { break };
                let Message::Text(t) = msg else {
                    if matches!(msg, Message::Close(_)) { break }
                    continue;
                };
                let v: Value = serde_json::from_str(&t).unwrap_or_default();
                match v["t"].as_str() {
                    Some("i") => input.write_all(v["d"].as_str().unwrap_or("").as_bytes()).await?,
                    Some("r") => {
                        let width = v["c"].as_u64().unwrap_or(120).clamp(10, 500) as u16;
                        let height = v["r"].as_u64().unwrap_or(32).clamp(5, 200) as u16;
                        let _ = st.docker.resize_exec(&exec.id, ResizeExecOptions { width, height }).await;
                    }
                    _ => {}
                }
            }
        }
    }
    let _ = socket.send(Message::Text("{\"t\":\"exit\"}".into())).await;
    Ok(())
}

// ---- deploy webhooks ---------------------------------------------------------

pub async fn hooks(State(st): State<AppState>, _u: AuthUser) -> ApiResult<Json<Value>> {
    let rows: Vec<(
        String,
        Option<String>,
        chrono::DateTime<chrono::Utc>,
        Option<chrono::DateTime<chrono::Utc>>,
    )> = sqlx::query_as(
        "SELECT project, created_by, created_at, last_used_at FROM deploy_hooks ORDER BY project",
    )
    .fetch_all(&st.db)
    .await?;
    Ok(Json(json!(rows
        .into_iter()
        .map(|(project, created_by, created_at, last_used_at)| json!({
            "project": project, "created_by": created_by, "created_at": created_at, "last_used_at": last_used_at,
        }))
        .collect::<Vec<_>>())))
}

/// Creates or rotates the secret; it is shown only in this response.
pub async fn create_hook(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    u.require_sudo()?;
    find_project(&st, &name).await?;
    let secret = random_token();
    sqlx::query(
        "INSERT INTO deploy_hooks (project, secret_enc, created_by) VALUES ($1, $2, $3)
         ON CONFLICT (project) DO UPDATE SET secret_enc = EXCLUDED.secret_enc, created_by = EXCLUDED.created_by,
                                             created_at = now(), last_used_at = NULL",
    )
    .bind(&name)
    .bind(st.cipher.encrypt(secret.as_bytes())?)
    .bind(&u.username)
    .execute(&st.db)
    .await?;
    audit::user_action(&st, &u, &client, "hook.create", &name, true, json!({})).await;
    Ok(Json(
        json!({ "project": name, "secret": secret, "path": format!("/api/hooks/deploy/{name}") }),
    ))
}

pub async fn delete_hook(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    u.require_sudo()?;
    sqlx::query("DELETE FROM deploy_hooks WHERE project = $1")
        .bind(&name)
        .execute(&st.db)
        .await?;
    audit::user_action(&st, &u, &client, "hook.delete", &name, true, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

const HOOK_SKEW: i64 = 300;

/// `POST /api/hooks/deploy/{project}` with
/// `X-Styk-Timestamp: <unix>` and `X-Styk-Signature: sha256=<hex hmac(secret, "<ts>.<body>")>`.
/// Body (optional): `{"services": ["api"]}`. Runs `pull` + `up -d`.
pub async fn deploy_hook(
    State(st): State<AppState>,
    client: ClientInfo,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    let hdr = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).unwrap_or("");
    let ts: i64 = hdr("x-styk-timestamp")
        .parse()
        .map_err(|_| ApiError::Unauthorized)?;
    if (chrono::Utc::now().timestamp() - ts).abs() > HOOK_SKEW {
        return Err(ApiError::Unauthorized);
    }
    let key = format!("hook:{name}");
    if st.limiter.blocked(&client.ip, Some(&key)) {
        return Err(ApiError::TooManyRequests);
    }
    let enc: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT secret_enc FROM deploy_hooks WHERE project = $1")
            .bind(&name)
            .fetch_optional(&st.db)
            .await?;
    let ok = match enc {
        Some(enc) => verify(
            &st.cipher.decrypt(&enc)?,
            ts,
            &body,
            hdr("x-styk-signature"),
        ),
        None => false,
    };
    if !ok {
        st.limiter.fail(&client.ip, Some(&key));
        audit::record(
            &st,
            audit::Entry {
                user_id: None,
                username: None,
                action: "hook.deploy",
                target: Some(&name),
                ip: &client.ip,
                ok: false,
                details: json!({ "reason": "bad signature" }),
            },
        )
        .await;
        return Err(ApiError::Unauthorized);
    }
    let services: Vec<String> = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|v| serde_json::from_value(v["services"].clone()).ok())
        .unwrap_or_default();
    let p = find_project(&st, &name).await?;
    if p.protected && (services.is_empty() || services.iter().any(|s| is_proxy(s))) {
        return Err(ApiError::Forbidden(
            "protected",
            "для защищённого проекта укажите services (кроме Caddy)".into(),
        ));
    }
    sqlx::query("UPDATE deploy_hooks SET last_used_at = now() WHERE project = $1")
        .bind(&name)
        .execute(&st.db)
        .await?;
    let task = compose_task(&p, ComposeAction::Redeploy, services)?;
    let actor = Actor {
        user_id: None,
        username: "webhook",
        ip: &client.ip,
    };
    let id = jobs::start(&st, actor, "compose", &name, task).await?;
    Ok(Json(json!({ "job": id })))
}

fn verify(secret: &[u8], ts: i64, body: &[u8], sig: &str) -> bool {
    let Some(hex_sig) = sig.strip_prefix("sha256=") else {
        return false;
    };
    let Ok(sig) = hex::decode(hex_sig) else {
        return false;
    };
    let mut mac = <Hmac<Sha256>>::new_from_slice(secret).expect("any key length");
    mac.update(ts.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    mac.verify_slice(&sig).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature() {
        let mut mac = <Hmac<Sha256>>::new_from_slice(b"k").unwrap();
        mac.update(b"100.{}");
        let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        assert!(verify(b"k", 100, b"{}", &sig));
        assert!(!verify(b"k", 101, b"{}", &sig));
        assert!(!verify(b"other", 100, b"{}", &sig));
        assert!(!verify(b"k", 100, b"{}", "deadbeef"));
    }
}
