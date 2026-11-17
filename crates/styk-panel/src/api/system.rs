use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::Response;
use axum::Json;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;

use crate::auth::AuthUser;
use crate::docker::{self, DiskUsage};
use crate::error::{ApiError, ApiResult};
use crate::metrics::{HostInfo, Sample};
use crate::state::AppState;

#[derive(Serialize)]
pub struct Overview {
    info: Option<HostInfo>,
    sample: Option<Arc<Sample>>,
    agent_error: Option<String>,
    docker: Option<DiskUsage>,
}

#[derive(Deserialize)]
pub struct OverviewQuery {
    /// Docker disk usage is only on the dashboard; the sidebar needs just the hostname.
    #[serde(default)]
    docker: bool,
}

pub async fn overview(
    State(st): State<AppState>,
    _u: AuthUser,
    Query(q): Query<OverviewQuery>,
) -> ApiResult<Json<Overview>> {
    let docker = if q.docker {
        docker::disk_usage(&st).await.ok()
    } else {
        None
    };
    Ok(Json(Overview {
        info: st.metrics.info().await,
        sample: st.metrics.latest().await,
        agent_error: st.metrics.error().await,
        docker,
    }))
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    range: String,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Point {
    ts: DateTime<Utc>,
    cpu_pct: f32,
    mem_used: i64,
    mem_total: i64,
    load1: f32,
    disk_used: i64,
    disk_read_bps: f32,
    disk_write_bps: f32,
    net_rx_bps: f32,
    net_tx_bps: f32,
}

/// `1h` comes from the in-memory 5 s ring, longer ranges from Postgres.
pub async fn history(
    State(st): State<AppState>,
    _u: AuthUser,
    Query(q): Query<HistoryQuery>,
) -> ApiResult<Json<Vec<Point>>> {
    let (span, res) = match q.range.as_str() {
        "1h" => {
            let since = (Utc::now() - Duration::hours(1)).timestamp_millis();
            let pts = st
                .metrics
                .recent(since)
                .await
                .iter()
                .map(|s| {
                    let root = s.filesystems.iter().find(|f| f.mount == "/");
                    Point {
                        ts: DateTime::from_timestamp_millis(s.ts).unwrap_or_default(),
                        cpu_pct: s.cpu_pct,
                        mem_used: s.mem_used as i64,
                        mem_total: s.mem_total as i64,
                        load1: s.load[0] as f32,
                        disk_used: root.map_or(0, |f| f.used as i64),
                        disk_read_bps: s.disk_read_bps,
                        disk_write_bps: s.disk_write_bps,
                        net_rx_bps: s.net_rx_bps,
                        net_tx_bps: s.net_tx_bps,
                    }
                })
                .collect();
            return Ok(Json(pts));
        }
        "6h" => (Duration::hours(6), "1m"),
        "24h" => (Duration::hours(24), "1m"),
        "7d" => (Duration::days(7), "1h"),
        "30d" => (Duration::days(30), "1h"),
        "90d" => (Duration::days(90), "1h"),
        _ => {
            return Err(ApiError::BadRequest(
                "range must be one of 1h, 6h, 24h, 7d, 30d, 90d".into(),
            ))
        }
    };
    let rows = sqlx::query_as::<_, Point>(
        "SELECT ts, cpu_pct, mem_used, mem_total, load1, disk_used, disk_read_bps, disk_write_bps, net_rx_bps, net_tx_bps
           FROM metrics_host WHERE resolution = $1 AND ts > $2 ORDER BY ts",
    )
    .bind(res)
    .bind(Utc::now() - span)
    .fetch_all(&st.db)
    .await?;
    Ok(Json(rows))
}

pub async fn live(State(st): State<AppState>, _u: AuthUser, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| stream_samples(st, socket))
}

async fn stream_samples(st: AppState, mut socket: WebSocket) {
    let mut rx = st.metrics.tx.subscribe();
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(sample) => {
                    let Ok(text) = serde_json::to_string(&*sample) else { continue };
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            },
        }
    }
}
