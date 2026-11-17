//! Host metrics from /proc and statvfs.

use std::collections::HashSet;
use std::fs;

use anyhow::{Context, Result};
use styk_proto::{CpuTimes, FsUsage, HostSnapshot, IoCounters, MemInfo, NetCounters};

const REAL_FS: &[&str] = &["ext4", "ext3", "xfs", "btrfs", "zfs", "f2fs", "vfat"];

pub fn hostname() -> String {
    fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

pub fn snapshot() -> Result<HostSnapshot> {
    Ok(HostSnapshot {
        ts_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64,
        hostname: hostname(),
        kernel: fs::read_to_string("/proc/sys/kernel/osrelease")
            .unwrap_or_default()
            .trim()
            .into(),
        os: os_name(),
        uptime_secs: fs::read_to_string("/proc/uptime")?
            .split_whitespace()
            .next()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0) as u64,
        cpu: cpu_times(&fs::read_to_string("/proc/stat")?),
        load: load(&fs::read_to_string("/proc/loadavg")?),
        mem: meminfo(&fs::read_to_string("/proc/meminfo")?),
        filesystems: filesystems()?,
        disk_io: diskstats(&fs::read_to_string("/proc/diskstats")?),
        net: netdev(&fs::read_to_string("/proc/net/dev")?),
    })
}

fn os_name() -> String {
    fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| {
                l.strip_prefix("PRETTY_NAME=")
                    .map(|v| v.trim_matches('"').to_string())
            })
        })
        .unwrap_or_default()
}

pub fn cpu_times(stat: &str) -> Vec<CpuTimes> {
    stat.lines()
        .filter(|l| l.starts_with("cpu"))
        .map(|l| {
            let v: Vec<u64> = l
                .split_whitespace()
                .skip(1)
                .filter_map(|x| x.parse().ok())
                .collect();
            // user nice system idle iowait irq softirq steal (guest is already in user)
            let total: u64 = v.iter().take(8).sum();
            let idle = v.get(3).copied().unwrap_or(0) + v.get(4).copied().unwrap_or(0);
            CpuTimes {
                busy: total.saturating_sub(idle),
                total,
            }
        })
        .collect()
}

pub fn load(s: &str) -> [f64; 3] {
    let mut it = s.split_whitespace().map(|x| x.parse().unwrap_or(0.0));
    [
        it.next().unwrap_or(0.0),
        it.next().unwrap_or(0.0),
        it.next().unwrap_or(0.0),
    ]
}

pub fn meminfo(s: &str) -> MemInfo {
    let get = |key: &str| -> u64 {
        s.lines()
            .find_map(|l| l.strip_prefix(key))
            .and_then(|r| r.trim_start_matches(':').split_whitespace().next())
            .and_then(|n| n.parse::<u64>().ok())
            .unwrap_or(0)
            * 1024
    };
    MemInfo {
        total: get("MemTotal"),
        available: get("MemAvailable"),
        swap_total: get("SwapTotal"),
        swap_free: get("SwapFree"),
    }
}

// statvfs field widths differ between platforms, so the casts are not redundant everywhere.
#[allow(clippy::unnecessary_cast)]
fn filesystems() -> Result<Vec<FsUsage>> {
    let mounts = fs::read_to_string("/proc/mounts").context("read /proc/mounts")?;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for line in mounts.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 3 || !REAL_FS.contains(&f[2]) || !seen.insert(f[0].to_string()) {
            continue;
        }
        let mount = f[1].replace("\\040", " ");
        if mount.starts_with("/var/lib/docker") || mount.starts_with("/snap") {
            continue;
        }
        let Ok(st) = nix::sys::statvfs::statvfs(mount.as_str()) else {
            continue;
        };
        let frsize = st.fragment_size() as u64;
        let total = st.blocks() as u64 * frsize;
        let free = st.blocks_free() as u64 * frsize;
        out.push(FsUsage {
            mount,
            device: f[0].into(),
            fstype: f[2].into(),
            total,
            used: total.saturating_sub(free),
            inodes_total: st.files() as u64,
            inodes_used: (st.files() - st.files_free()) as u64,
        });
    }
    Ok(out)
}

/// Whole physical disks only (sda, vda, nvme0n1), partitions would double count.
fn is_whole_disk(name: &str) -> bool {
    if let Some(rest) = name.strip_prefix("nvme") {
        return rest.contains('n') && !rest.contains('p');
    }
    ["sd", "vd", "xvd", "hd"]
        .iter()
        .any(|p| name.starts_with(p))
        && !name.chars().last().is_some_and(|c| c.is_ascii_digit())
}

pub fn diskstats(s: &str) -> IoCounters {
    let mut io = IoCounters::default();
    for l in s.lines() {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 10 || !is_whole_disk(f[2]) {
            continue;
        }
        io.read_bytes += f[5].parse::<u64>().unwrap_or(0) * 512;
        io.write_bytes += f[9].parse::<u64>().unwrap_or(0) * 512;
    }
    io
}

pub fn netdev(s: &str) -> NetCounters {
    let mut n = NetCounters::default();
    for l in s.lines().skip(2) {
        let Some((iface, rest)) = l.split_once(':') else {
            continue;
        };
        let iface = iface.trim();
        if iface == "lo"
            || ["veth", "br-", "docker", "virbr", "tun", "wg"]
                .iter()
                .any(|p| iface.starts_with(p))
        {
            continue;
        }
        let f: Vec<u64> = rest
            .split_whitespace()
            .filter_map(|x| x.parse().ok())
            .collect();
        n.rx_bytes += f.first().copied().unwrap_or(0);
        n.tx_bytes += f.get(8).copied().unwrap_or(0);
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cpu() {
        let s = "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 50 0 25 400 25 0 0 0 0 0\nintr 1\n";
        let c = cpu_times(s);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].total, 1000);
        assert_eq!(c[0].busy, 150);
    }

    #[test]
    fn whole_disks() {
        assert!(is_whole_disk("sda"));
        assert!(!is_whole_disk("sda1"));
        assert!(is_whole_disk("nvme0n1"));
        assert!(!is_whole_disk("nvme0n1p2"));
        assert!(!is_whole_disk("loop0"));
    }

    #[test]
    fn net_skips_virtual() {
        let s = "h1\nh2\n  eth0: 100 0 0 0 0 0 0 0 200 0 0 0 0 0 0 0\nveth1: 5 0 0 0 0 0 0 0 5 0 0 0 0 0 0 0\n";
        let n = netdev(s);
        assert_eq!((n.rx_bytes, n.tx_bytes), (100, 200));
    }
}
