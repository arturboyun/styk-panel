//! Per-container usage from cgroup v2, much cheaper than the Docker stats API.

use std::fs;
use std::path::PathBuf;

use styk_proto::{ContainerCgroup, IoCounters};

fn cgroup_dir(id: &str) -> Option<PathBuf> {
    let root = std::env::var("STYK_CGROUP_ROOT").unwrap_or_else(|_| "/sys/fs/cgroup".into());
    [
        format!("{root}/system.slice/docker-{id}.scope"),
        format!("{root}/docker/{id}"),
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|p| p.is_dir())
}

fn read_u64(p: PathBuf) -> Option<u64> {
    fs::read_to_string(p).ok()?.trim().parse().ok()
}

pub fn stats(ids: &[String]) -> Vec<ContainerCgroup> {
    ids.iter()
        .filter(|id| id.len() == 64 && id.chars().all(|c| c.is_ascii_hexdigit()))
        .filter_map(|id| {
            let dir = cgroup_dir(id)?;
            let cpu = fs::read_to_string(dir.join("cpu.stat")).ok()?;
            let cpu_usage_usec = cpu
                .lines()
                .find_map(|l| l.strip_prefix("usage_usec "))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            let mut io = IoCounters::default();
            for l in fs::read_to_string(dir.join("io.stat"))
                .unwrap_or_default()
                .lines()
            {
                for kv in l.split_whitespace().skip(1) {
                    match kv.split_once('=') {
                        Some(("rbytes", v)) => io.read_bytes += v.parse().unwrap_or(0),
                        Some(("wbytes", v)) => io.write_bytes += v.parse().unwrap_or(0),
                        _ => {}
                    }
                }
            }
            Some(ContainerCgroup {
                id: id.clone(),
                cpu_usage_usec,
                mem_current: read_u64(dir.join("memory.current")).unwrap_or(0),
                mem_max: read_u64(dir.join("memory.max")),
                io,
                pids: read_u64(dir.join("pids.current")).unwrap_or(0),
            })
        })
        .collect()
}
