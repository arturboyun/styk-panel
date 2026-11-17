//! Versions and updates: the newest GitHub release, `styk update` through the agent, and
//! the panel's own daily database dump.

use std::time::Duration;

use axum::extract::{Query, State};
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use styk_proto::{Method, Task};

use crate::audit;
use crate::auth::{AuthUser, ClientInfo};
use crate::error::{agent_error, ApiError, ApiResult};
use crate::jobs;
use crate::settings;
use crate::state::AppState;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const LATEST_KEY: &str = "update:latest";
const LATEST_TTL: Duration = Duration::from_secs(6 * 3600);
const SELF_BACKUP_KEY: &str = "self_backup.last";

/// `v1.2.3` or `1.2.3` (pre-release suffix ignored) as a comparable tuple.
fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim_start_matches('v').split('-').next()?;
    let mut it = core.split('.').map(|p| p.parse::<u64>().ok());
    let t = (it.next()??, it.next()??, it.next()??);
    it.next().is_none().then_some(t)
}

fn newer(latest: &str, current: &str) -> bool {
    matches!((parse(latest), parse(current)), (Some(l), Some(c)) if l > c)
}

/// The newest published (non-draft, non-prerelease) release of STYK_UPDATE_REPO.
async fn latest(st: &AppState, force: bool) -> Option<Value> {
    let repo = st.cfg.update_repo.trim();
    if repo.is_empty() {
        return None;
    }
    if force {
        st.cache.invalidate(LATEST_KEY);
    }
    let res = st
        .cache
        .get_or(LATEST_KEY, LATEST_TTL, || async {
            let r: Value = st
                .http
                .get(format!(
                    "https://api.github.com/repos/{repo}/releases/latest"
                ))
                .header("accept", "application/vnd.github+json")
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            let notes: String = r["body"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(4000)
                .collect();
            Ok::<_, reqwest::Error>(json!({
                "version": r["tag_name"],
                "url": r["html_url"],
                "published_at": r["published_at"],
                "notes": notes,
            }))
        })
        .await;
    match res {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!("release check: {e}");
            None
        }
    }
}

#[derive(Deserialize)]
pub struct VersionQuery {
    #[serde(default)]
    check: bool,
}

pub async fn version(
    State(st): State<AppState>,
    _u: AuthUser,
    Query(q): Query<VersionQuery>,
) -> ApiResult<Json<Value>> {
    let (latest, status) = tokio::join!(
        latest(&st, q.check),
        st.agent.call::<Value>(Method::UpdateStatus)
    );
    let available = latest
        .as_ref()
        .and_then(|l| l["version"].as_str())
        .is_some_and(|l| newer(l, VERSION));
    Ok(Json(json!({
        "panel": VERSION,
        "repo": st.cfg.update_repo,
        "latest": latest,
        "update_available": available,
        // Old agents answer "unknown method": show the panel version only.
        "update": status.ok(),
    })))
}

#[derive(Deserialize)]
pub struct UpdateReq {
    version: String,
}

pub async fn update(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Json(req): Json<UpdateReq>,
) -> ApiResult<Json<Value>> {
    u.require_sudo()?;
    let res = st
        .agent
        .call::<Value>(Method::Update {
            version: req.version.clone(),
        })
        .await;
    audit::user_action(
        &st,
        &u,
        &client,
        "system.update",
        &req.version,
        res.is_ok(),
        json!({ "from": VERSION }),
    )
    .await;
    Ok(Json(res.map_err(agent_error)?))
}

/// Daily `pg_dump` of the panel's own database (STYK_DB_CONTAINER) into
/// /var/backups/styk-panel, 14 days kept by the agent.
pub fn spawn(st: AppState) {
    if st.cfg.db_container.is_empty() {
        return;
    }
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(900)).await;
            let last: i64 = settings::get(&st, SELF_BACKUP_KEY).await.unwrap_or(0);
            let now = Utc::now().timestamp();
            if now - last < 23 * 3600 {
                continue;
            }
            let task = Task::PgDump {
                container: st.cfg.db_container.clone(),
                database: "styk".into(),
                user: "styk".into(),
                daily: true,
            };
            let actor = jobs::Actor {
                user_id: None,
                username: "system",
                ip: "",
            };
            let target = st.cfg.db_container.clone();
            match jobs::start(&st, actor, "pg", &target, task).await {
                Ok(id) => tracing::info!(id, "panel database dump started"),
                // A busy or failing dump is retried on the next pass; a gone container
                // would log every 15 min, so it also waits a day.
                Err(ApiError::BadRequest(m)) => tracing::info!("panel database dump: {m}"),
                Err(e) => tracing::warn!("panel database dump: {e}"),
            }
            if let Err(e) = settings::set(&st, SELF_BACKUP_KEY, &now).await {
                tracing::warn!("self backup marker: {e}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(newer("v0.2.0", "0.1.9"));
        assert!(newer("v1.0.0", "0.10.3"));
        assert!(!newer("v0.1.0", "0.1.0"));
        assert!(!newer("v0.1.0-rc.1", "0.1.0"));
        assert!(!newer("garbage", "0.1.0"));
        assert_eq!(parse("v1.2"), None);
    }
}
