//! styk-agent: privileged host-side helper for styk-panel.
//!
//! Listens on a unix socket only (no network), accepts the typed methods from
//! `styk_proto::Method` and nothing else.

mod access;
mod cgroup;
mod fs;
mod host;
mod security;
mod systemd;
mod tasks;
mod update;
mod util;

use std::os::unix::fs::PermissionsExt;

use anyhow::{Context, Result};
use styk_proto::{Method, Reply, ReplyResult, Request};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let path =
        std::env::var("STYK_AGENT_SOCKET").unwrap_or_else(|_| styk_proto::DEFAULT_SOCKET.into());
    if let Some(dir) = std::path::Path::new(&path).parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).with_context(|| format!("bind {path}"))?;

    // Socket is reachable by root and by the group the panel container runs with.
    if let Ok(gid) = std::env::var("STYK_AGENT_GID") {
        let gid: u32 = gid.parse().context("STYK_AGENT_GID must be numeric")?;
        nix::unistd::chown(path.as_str(), None, Some(nix::unistd::Gid::from_raw(gid)))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o660))?;
    } else {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    tracing::info!(%path, "styk-agent listening");

    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(async move {
            if let Err(e) = serve(stream).await {
                tracing::debug!("connection closed: {e:#}");
            }
        });
    }
}

async fn serve(stream: UnixStream) -> Result<()> {
    let (rd, mut wr) = stream.into_split();
    let mut lines = BufReader::new(rd).lines();
    while let Some(line) = lines.next_line().await? {
        let reply = match serde_json::from_str::<Request>(&line) {
            Ok(req) => Reply {
                id: req.id,
                result: dispatch(req.method).await,
            },
            Err(e) => Reply {
                id: 0,
                result: ReplyResult::Err(format!("bad request: {e}")),
            },
        };
        let mut out = serde_json::to_vec(&reply)?;
        out.push(b'\n');
        wr.write_all(&out).await?;
    }
    Ok(())
}

async fn dispatch(method: Method) -> ReplyResult {
    let res = tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        Ok(match method {
            Method::Ping => serde_json::to_value(styk_proto::Pong {
                version: env!("CARGO_PKG_VERSION").into(),
                hostname: host::hostname(),
            })?,
            Method::HostSnapshot => serde_json::to_value(host::snapshot()?)?,
            Method::ContainerStats { ids } => serde_json::to_value(cgroup::stats(&ids))?,
            Method::SshActivity { hours } => to(security::ssh_activity(hours)?)?,
            Method::ListeningPorts => to(security::listening()?)?,
            Method::Firewall => to(security::firewall()?)?,
            Method::AccessGet => to(access::get()?)?,
            Method::AccessSet { config, force } => to(access::set(config, force)?)?,
            Method::SshdConfig => to(security::sshd_config()?)?,
            Method::SecurityTools => to(security::tools())?,
            Method::Updates => to(security::updates()?)?,
            Method::Timers => to(systemd::timers()?)?,
            Method::UnitInfo { unit } => to(systemd::unit_info(&unit)?)?,
            Method::UnitJournal { unit, lines } => to(systemd::unit_journal(&unit, lines)?)?,
            Method::Backups => to(serde_json::json!({
                "dirs": systemd::backup_dirs(),
                "files": systemd::backups(),
            }))?,
            Method::BackupStatus => to(systemd::backup_status())?,
            Method::TimerSchedule { timer, on_calendar } => {
                to(systemd::set_schedule(&timer, on_calendar.as_deref())?)?
            }
            Method::Update { version } => update::start(&version)?,
            Method::UpdateStatus => update::status(),
            Method::FsList { path } => to(fs::sandbox().list(&path)?)?,
            Method::FsStat { path } => to(fs::sandbox().stat(&path)?)?,
            Method::FsRead { path, offset, len } => to(fs::sandbox().read(&path, offset, len)?)?,
            Method::FsWrite {
                path,
                content_b64,
                in_place,
                expected_mtime,
            } => to(fs::sandbox().write(&path, &content_b64, in_place, expected_mtime)?)?,
            Method::FsUpload {
                path,
                offset,
                data_b64,
                last,
            } => to(fs::sandbox().upload(&path, offset, &data_b64, last)?)?,
            Method::FsMkdir { path } => to(fs::sandbox().mkdir(&path)?)?,
            Method::FsRename { from, to: dest } => to(fs::sandbox().rename(&from, &dest)?)?,
            Method::FsTrash { path } => to(fs::sandbox().trash(&path)?)?,
            Method::FsHistory { path } => to(fs::sandbox().history(&path)?)?,
            Method::TaskStart { task } => {
                tracing::info!(?task, "task start");
                to(tasks::start(task)?)?
            }
            Method::TaskPoll { id, offset } => to(tasks::poll(id, offset)?)?,
        })
    })
    .await;
    match res {
        Ok(Ok(v)) => ReplyResult::Ok(v),
        Ok(Err(e)) => ReplyResult::Err(format!("{e:#}")),
        Err(e) => ReplyResult::Err(format!("task failed: {e}")),
    }
}

fn to<T: serde::Serialize>(v: T) -> serde_json::Result<serde_json::Value> {
    serde_json::to_value(v)
}
