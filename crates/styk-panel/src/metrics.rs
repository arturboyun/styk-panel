//! Samples the host every 5 s via styk-agent, keeps one hour in memory for
//! live charts and writes 1-minute / 1-hour aggregates to Postgres.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use bollard::container::ListContainersOptions;
use chrono::{DateTime, TimeZone, Timelike, Utc};
use serde::Serialize;
use styk_proto::{ContainerCgroup, FsUsage, HostSnapshot, Method};
use tokio::sync::{broadcast, RwLock};

use crate::state::AppState;

pub const INTERVAL: Duration = Duration::from_secs(5);
const RING: usize = 720; // one hour at 5 s

#[derive(Debug, Clone, Serialize)]
pub struct HostInfo {
    pub hostname: String,
    pub os: String,
    pub kernel: String,
    pub cores: usize,
    pub uptime_secs: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Sample {
    pub ts: i64,
    pub cpu_pct: f32,
    pub cores: Vec<f32>,
    pub mem_used: u64,
    pub mem_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub load: [f64; 3],
    pub disk_read_bps: f32,
    pub disk_write_bps: f32,
    pub net_rx_bps: f32,
    pub net_tx_bps: f32,
    pub filesystems: Vec<FsUsage>,
    pub containers: Vec<ContainerSample>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContainerSample {
    pub id: String,
    pub name: String,
    pub project: Option<String>,
    /// Percent of one core, like `docker stats`.
    pub cpu_pct: f32,
    pub mem_used: u64,
    pub mem_limit: Option<u64>,
    pub pids: u64,
}

#[derive(Default)]
struct Inner {
    samples: VecDeque<Arc<Sample>>,
    info: Option<HostInfo>,
    prev: Option<(HostSnapshot, HashMap<String, ContainerCgroup>)>,
    error: Option<String>,
}

pub struct Metrics {
    inner: RwLock<Inner>,
    pub tx: broadcast::Sender<Arc<Sample>>,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            inner: RwLock::default(),
            tx: broadcast::channel(16).0,
        }
    }
}

impl Metrics {
    pub async fn latest(&self) -> Option<Arc<Sample>> {
        self.inner.read().await.samples.back().cloned()
    }
    pub async fn info(&self) -> Option<HostInfo> {
        self.inner.read().await.info.clone()
    }
    pub async fn error(&self) -> Option<String> {
        self.inner.read().await.error.clone()
    }
    pub async fn recent(&self, since_ms: i64) -> Vec<Arc<Sample>> {
        self.inner
            .read()
            .await
            .samples
            .iter()
            .filter(|s| s.ts >= since_ms)
            .cloned()
            .collect()
    }
}

fn rate(cur: u64, prev: u64, secs: f64) -> f32 {
    if secs <= 0.0 {
        0.0
    } else {
        (cur.saturating_sub(prev) as f64 / secs) as f32
    }
}

fn pct(busy: u64, pbusy: u64, total: u64, ptotal: u64) -> f32 {
    let dt = total.saturating_sub(ptotal);
    if dt == 0 {
        0.0
    } else {
        (busy.saturating_sub(pbusy) as f64 / dt as f64 * 100.0) as f32
    }
}

pub fn spawn(state: AppState) {
    let st = state.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            if let Err(e) = collect(&st).await {
                tracing::warn!("metrics: {e:#}");
                st.metrics.inner.write().await.error = Some(format!("{e:#}"));
            }
        }
    });
    tokio::spawn(async move {
        let mut last_minute = Utc::now().timestamp() / 60;
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let minute = Utc::now().timestamp() / 60;
            if minute == last_minute {
                continue;
            }
            last_minute = minute;
            let start = Utc.timestamp_opt((minute - 1) * 60, 0).unwrap();
            if let Err(e) = persist_minute(&state, start).await {
                tracing::warn!("metrics persist: {e:#}");
            }
            if Utc::now().minute() == 0 {
                if let Err(e) = rollup_hour(&state).await {
                    tracing::warn!("metrics rollup: {e:#}");
                }
            }
        }
    });
}

async fn collect(state: &AppState) -> Result<()> {
    let snap: HostSnapshot = state.agent.call(Method::HostSnapshot).await?;
    let running = state
        .docker
        .list_containers(Some(ListContainersOptions::<String> {
            all: false,
            ..Default::default()
        }))
        .await?;
    let ids: Vec<String> = running.iter().filter_map(|c| c.id.clone()).collect();
    let cgroups: Vec<ContainerCgroup> = state.agent.call(Method::ContainerStats { ids }).await?;
    let cgroups: HashMap<String, ContainerCgroup> =
        cgroups.into_iter().map(|c| (c.id.clone(), c)).collect();

    let mut inner = state.metrics.inner.write().await;
    inner.info = Some(HostInfo {
        hostname: snap.hostname.clone(),
        os: snap.os.clone(),
        kernel: snap.kernel.clone(),
        cores: snap.cpu.len().saturating_sub(1),
        uptime_secs: snap.uptime_secs,
    });
    inner.error = None;

    let Some((prev, prev_cg)) = inner.prev.replace((snap.clone(), cgroups.clone())) else {
        return Ok(()); // first tick only primes the counters
    };
    let secs = (snap.ts_ms.saturating_sub(prev.ts_ms)) as f64 / 1000.0;
    let cpu: Vec<f32> = snap
        .cpu
        .iter()
        .zip(prev.cpu.iter())
        .map(|(c, p)| pct(c.busy, p.busy, c.total, p.total))
        .collect();

    let containers = running
        .iter()
        .filter_map(|c| {
            let id = c.id.clone()?;
            let cur = cgroups.get(&id)?;
            let cpu_pct = prev_cg
                .get(&id)
                .map(|p| {
                    (cur.cpu_usage_usec.saturating_sub(p.cpu_usage_usec) as f64 / (secs * 1e6)
                        * 100.0) as f32
                })
                .unwrap_or(0.0);
            let labels = c.labels.clone().unwrap_or_default();
            Some(ContainerSample {
                name: c
                    .names
                    .as_ref()?
                    .first()?
                    .trim_start_matches('/')
                    .to_string(),
                project: labels.get("com.docker.compose.project").cloned(),
                id,
                cpu_pct,
                mem_used: cur.mem_current,
                mem_limit: cur.mem_max,
                pids: cur.pids,
            })
        })
        .collect();

    let sample = Arc::new(Sample {
        ts: snap.ts_ms as i64,
        cpu_pct: cpu.first().copied().unwrap_or(0.0),
        cores: cpu.into_iter().skip(1).collect(),
        mem_used: snap.mem.total.saturating_sub(snap.mem.available),
        mem_total: snap.mem.total,
        swap_used: snap.mem.swap_total.saturating_sub(snap.mem.swap_free),
        swap_total: snap.mem.swap_total,
        load: snap.load,
        disk_read_bps: rate(snap.disk_io.read_bytes, prev.disk_io.read_bytes, secs),
        disk_write_bps: rate(snap.disk_io.write_bytes, prev.disk_io.write_bytes, secs),
        net_rx_bps: rate(snap.net.rx_bytes, prev.net.rx_bytes, secs),
        net_tx_bps: rate(snap.net.tx_bytes, prev.net.tx_bytes, secs),
        filesystems: snap.filesystems.clone(),
        containers,
    });
    inner.samples.push_back(sample.clone());
    while inner.samples.len() > RING {
        inner.samples.pop_front();
    }
    drop(inner);
    let _ = state.metrics.tx.send(sample);
    Ok(())
}

fn root_fs(s: &Sample) -> (u64, u64) {
    s.filesystems
        .iter()
        .find(|f| f.mount == "/")
        .or_else(|| s.filesystems.iter().max_by_key(|f| f.total))
        .map_or((0, 0), |f| (f.used, f.total))
}

async fn persist_minute(state: &AppState, start: DateTime<Utc>) -> Result<()> {
    let from = start.timestamp_millis();
    let samples: Vec<_> = state
        .metrics
        .recent(from)
        .await
        .into_iter()
        .filter(|s| s.ts < from + 60_000)
        .collect();
    if samples.is_empty() {
        return Ok(());
    }
    let n = samples.len() as f32;
    let avg = |f: fn(&Sample) -> f32| samples.iter().map(|s| f(s)).sum::<f32>() / n;
    let last = samples.last().unwrap();
    let (disk_used, disk_total) = root_fs(last);
    sqlx::query(
        "INSERT INTO metrics_host VALUES ('1m',$1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) ON CONFLICT DO NOTHING",
    )
    .bind(start)
    .bind(avg(|s| s.cpu_pct))
    .bind((samples.iter().map(|s| s.mem_used as f64).sum::<f64>() / n as f64) as i64)
    .bind(last.mem_total as i64)
    .bind(last.swap_used as i64)
    .bind(avg(|s| s.load[0] as f32))
    .bind(disk_used as i64)
    .bind(disk_total as i64)
    .bind(avg(|s| s.disk_read_bps))
    .bind(avg(|s| s.disk_write_bps))
    .bind(avg(|s| s.net_rx_bps))
    .bind(avg(|s| s.net_tx_bps))
    .execute(&state.db)
    .await?;

    let mut per: HashMap<&str, (Option<&str>, f32, u64, u32)> = HashMap::new();
    for s in &samples {
        for c in &s.containers {
            let e = per
                .entry(&c.name)
                .or_insert((c.project.as_deref(), 0.0, 0, 0));
            e.1 += c.cpu_pct;
            e.2 += c.mem_used;
            e.3 += 1;
        }
    }
    let (mut names, mut projects, mut cpus, mut mems) = (vec![], vec![], vec![], vec![]);
    for (name, (project, cpu, mem, k)) in per {
        names.push(name.to_string());
        projects.push(project.map(str::to_string));
        cpus.push(cpu / k as f32);
        mems.push((mem / k as u64) as i64);
    }
    sqlx::query(
        "INSERT INTO metrics_container (resolution, ts, name, project, cpu_pct, mem_used)
         SELECT '1m', $1, * FROM UNNEST($2::text[], $3::text[], $4::real[], $5::bigint[])
         ON CONFLICT DO NOTHING",
    )
    .bind(start)
    .bind(&names)
    .bind(&projects)
    .bind(&cpus)
    .bind(&mems)
    .execute(&state.db)
    .await?;
    Ok(())
}

async fn rollup_hour(state: &AppState) -> Result<()> {
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO metrics_host
         SELECT '1h', date_trunc('hour', ts), avg(cpu_pct), avg(mem_used)::bigint, max(mem_total), max(swap_used),
                avg(load1), max(disk_used), max(disk_total), avg(disk_read_bps), avg(disk_write_bps),
                avg(net_rx_bps), avg(net_tx_bps)
           FROM metrics_host
          WHERE resolution = '1m' AND ts >= date_trunc('hour', now()) - interval '1 hour' AND ts < date_trunc('hour', now())
          GROUP BY 2 ON CONFLICT DO NOTHING",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO metrics_container
         SELECT '1h', date_trunc('hour', ts), name, max(project), avg(cpu_pct), avg(mem_used)::bigint
           FROM metrics_container
          WHERE resolution = '1m' AND ts >= date_trunc('hour', now()) - interval '1 hour' AND ts < date_trunc('hour', now())
          GROUP BY 2, 3 ON CONFLICT DO NOTHING",
    )
    .execute(&mut *tx)
    .await?;
    for (table, res, keep) in [
        ("metrics_host", "1m", "7 days"),
        ("metrics_host", "1h", "90 days"),
        ("metrics_container", "1m", "7 days"),
        ("metrics_container", "1h", "90 days"),
    ] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE resolution = $1 AND ts < now() - $2::interval"
        ))
        .bind(res)
        .bind(keep)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}
