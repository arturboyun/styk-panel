//! HTTP uptime checks: one per site from the Caddyfile (kept in sync) plus manual URLs.
//! Checked every minute from inside the panel, so requests take the public path
//! (DNS → Cloudflare → Caddy). Anything below 500 counts as up.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use chrono::Utc;

use crate::state::AppState;

const RETENTION_DAYS: i32 = 30;
const SYNC_EVERY: Duration = Duration::from_secs(600);

pub fn spawn(st: AppState) {
    tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::limited(3))
            .user_agent("styk-panel-uptime")
            .build()
            .expect("http client");
        let mut last_sync: Option<Instant> = None;
        let mut last_cleanup = Instant::now();
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            if last_sync.is_none_or(|t| t.elapsed() > SYNC_EVERY) {
                if let Err(e) = sync(&st).await {
                    tracing::warn!("uptime sync: {e:#}");
                }
                last_sync = Some(Instant::now());
            }
            if let Err(e) = run(&st, &client).await {
                tracing::warn!("uptime: {e:#}");
            }
            if last_cleanup.elapsed() > Duration::from_secs(3600) {
                let _ = sqlx::query(
                    "DELETE FROM uptime_results WHERE ts < now() - make_interval(days => $1)",
                )
                .bind(RETENTION_DAYS)
                .execute(&st.db)
                .await;
                last_cleanup = Instant::now();
            }
        }
    });
}

/// Adds checks for new Caddy sites and drops the ones whose site is gone.
async fn sync(st: &AppState) -> anyhow::Result<()> {
    let (text, _) = crate::api::domains::read_caddyfile(st)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let env = crate::api::domains::caddy_env(st).await;
    let urls: HashSet<String> = crate::caddyfile::sites(&crate::caddyfile::parse(&text), &env)
        .into_iter()
        .filter(|s| s.redirect.is_none())
        .flat_map(|s| s.hosts)
        .filter(|h| crate::caddyfile::valid_domain(h) && !h.starts_with("*."))
        .map(|h| format!("https://{h}"))
        .collect();
    for u in &urls {
        sqlx::query("INSERT INTO uptime_checks (url, name, source) VALUES ($1, $2, 'caddy') ON CONFLICT (url) DO NOTHING")
            .bind(u)
            .bind(u.trim_start_matches("https://"))
            .execute(&st.db)
            .await?;
    }
    let existing: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, url FROM uptime_checks WHERE source = 'caddy'")
            .fetch_all(&st.db)
            .await?;
    for (id, url) in existing {
        if !urls.contains(&url) {
            sqlx::query("DELETE FROM uptime_checks WHERE id = $1")
                .bind(id)
                .execute(&st.db)
                .await?;
        }
    }
    Ok(())
}

async fn run(st: &AppState, client: &reqwest::Client) -> anyhow::Result<()> {
    let checks: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, url FROM uptime_checks WHERE enabled")
            .fetch_all(&st.db)
            .await?;
    let ts = Utc::now();
    let results = futures::future::join_all(checks.into_iter().map(|(id, url)| async move {
        let started = Instant::now();
        let res = client.get(&url).send().await;
        let ms = started.elapsed().as_millis() as i32;
        match res {
            Ok(r) => {
                let code = r.status().as_u16() as i32;
                (id, code < 500, Some(code), ms, None)
            }
            Err(e) => {
                let why = if e.is_timeout() {
                    "timeout".to_string()
                } else if e.is_connect() {
                    "connection failed".to_string()
                } else {
                    e.to_string().chars().take(200).collect()
                };
                (id, false, None, ms, Some(why))
            }
        }
    }))
    .await;
    for (id, up, status, ms, err) in results {
        sqlx::query("INSERT INTO uptime_results (check_id, ts, up, status, latency_ms, error) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING")
            .bind(id)
            .bind(ts)
            .bind(up)
            .bind(status)
            .bind(ms)
            .bind(err)
            .execute(&st.db)
            .await?;
    }
    Ok(())
}
