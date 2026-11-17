//! systemd timers/units and the backup directories they fill.

use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use styk_proto::{BackupFile, Timer};

use crate::util::{check_unit, env_list, run, DEFAULT_UNITS};

#[derive(Deserialize)]
struct RawTimer {
    unit: String,
    activates: Option<String>,
    next: Option<u64>,
    last: Option<u64>,
}

pub fn timers() -> Result<Vec<Timer>> {
    parse_timers(&run(
        "systemctl",
        &["list-timers", "--all", "--output=json"],
    )?)
}

fn parse_timers(out: &str) -> Result<Vec<Timer>> {
    let raw: Vec<RawTimer> = serde_json::from_str(out)?;
    let mut t: Vec<Timer> = raw
        .into_iter()
        .map(|r| Timer {
            unit: r.unit,
            activates: r.activates.unwrap_or_default(),
            next: r.next.filter(|&v| v > 0),
            last: r.last.filter(|&v| v > 0),
        })
        .collect();
    t.sort_by(|a, b| a.unit.cmp(&b.unit));
    Ok(t)
}

const UNIT_PROPS: &str = "Id,Description,LoadState,ActiveState,SubState,Result,UnitFileState,\
ExecMainStartTimestamp,ExecMainExitTimestamp,ExecMainStatus,ActiveEnterTimestamp,\
InactiveEnterTimestamp,NRestarts,Type,FragmentPath,MemoryCurrent";

pub fn unit_info(unit: &str) -> Result<BTreeMap<String, String>> {
    check_unit(unit)?;
    let p = format!("-p{UNIT_PROPS}");
    let out = run("systemctl", &["show", unit, "--timestamp=unix", &p, "--"])?;
    Ok(out
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.trim_start_matches('@').to_string()))
        .collect())
}

pub fn unit_journal(unit: &str, lines: u32) -> Result<Vec<String>> {
    check_unit(unit)?;
    let n = lines.clamp(1, 5000).to_string();
    let out = run(
        "journalctl",
        &["-u", unit, "-n", &n, "-o", "short-iso", "--no-pager", "-q"],
    )?;
    Ok(out.lines().map(str::to_string).collect())
}

pub fn backup_dirs() -> Vec<String> {
    env_list(
        "STYK_AGENT_BACKUP_DIRS",
        "/var/backups/containers,/var/backups/styk-panel",
    )
}

pub fn backups() -> Vec<BackupFile> {
    let mut out = Vec::new();
    for dir in backup_dirs() {
        walk(Path::new(&dir), 0, &mut out);
    }
    out.sort_by_key(|f| std::cmp::Reverse(f.mtime));
    out.truncate(5000);
    out
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<BackupFile>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let Ok(md) = e.path().symlink_metadata() else {
            continue;
        };
        if md.is_dir() && depth < 3 {
            walk(&e.path(), depth + 1, out);
        } else if md.is_file() {
            out.push(BackupFile {
                path: e.path().to_string_lossy().into_owned(),
                size: md.len(),
                mtime: md.mtime(),
            });
        }
    }
}

const STATUS_DIR: &str = "/var/lib/backup-all";

/// The JSON summaries written by backup-r2 and backup-restore-test; missing or
/// unreadable files come back as null.
pub fn backup_status() -> serde_json::Value {
    let read = |name: &str| {
        std::fs::read_to_string(format!("{STATUS_DIR}/{name}"))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
    };
    serde_json::json!({
        "r2": read("r2-status.json"),
        "restore_test": read("restore-test.json"),
    })
}

/// Points `timer` at a new OnCalendar (validated by systemd-analyze) via a drop-in,
/// or stops and disables it. Returns the timer's next run as `systemctl list-timers` sees it.
pub fn set_schedule(timer: &str, on_calendar: Option<&str>) -> Result<Option<Timer>> {
    check_unit(timer)?;
    let Some(service) = timer.strip_suffix(".timer").map(|b| format!("{b}.service")) else {
        bail!("{timer} is not a timer");
    };
    if !env_list("STYK_AGENT_UNITS", DEFAULT_UNITS).contains(&service) {
        bail!("{service} is not in STYK_AGENT_UNITS");
    }
    match on_calendar {
        None => {
            run("systemctl", &["disable", "--now", timer])?;
        }
        Some(cal) => {
            let cal = cal.trim();
            if cal.is_empty() || cal.len() > 200 || cal.contains(['\n', '\r']) {
                bail!("invalid OnCalendar: {cal:?}");
            }
            run("systemd-analyze", &["calendar", "--", cal])
                .with_context(|| format!("OnCalendar {cal:?} is not valid"))?;
            let dir = format!("/etc/systemd/system/{timer}.d");
            std::fs::create_dir_all(&dir)?;
            std::fs::write(
                format!("{dir}/schedule.conf"),
                format!("# Written by styk-agent (Styk Panel).\n[Timer]\nOnCalendar=\nOnCalendar={cal}\n"),
            )?;
            run("systemctl", &["daemon-reload"])?;
            run("systemctl", &["enable", timer])?;
            run("systemctl", &["restart", timer])?;
        }
    }
    Ok(timers()?.into_iter().find(|t| t.unit == timer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timers_json() {
        let out = r#"[{"next":1793724600000000,"left":1793724600000000,"last":1793724000831370,"passed":4519092428921,"unit":"sysstat-collect.timer","activates":"sysstat-collect.service"},{"next":0,"left":0,"last":null,"passed":null,"unit":"apt-daily.timer","activates":"apt-daily.service"}]"#;
        let t = parse_timers(out).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].unit, "apt-daily.timer");
        assert_eq!(t[0].next, None);
        assert_eq!(t[1].last, Some(1793724000831370));
    }
}
