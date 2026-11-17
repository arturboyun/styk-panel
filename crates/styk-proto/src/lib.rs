//! Wire protocol between styk-panel and styk-agent.
//!
//! Transport: unix socket, one JSON object per line. The panel sends a
//! [`Request`] envelope and the agent answers with a [`Reply`] envelope that
//! carries the same `id`. The agent only accepts the methods listed in
//! [`Method`]; there is deliberately no "run this command" call.

use serde::{Deserialize, Serialize};

pub const DEFAULT_SOCKET: &str = "/run/styk-agent/agent.sock";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    #[serde(flatten)]
    pub method: Method,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Method {
    Ping,
    HostSnapshot,
    ContainerStats {
        ids: Vec<String>,
    },

    // System and security (read-only).
    SshActivity {
        hours: u32,
    },
    ListeningPorts,
    Firewall,
    SshdConfig,
    SecurityTools,
    Updates,
    Timers,
    UnitInfo {
        unit: String,
    },
    UnitJournal {
        unit: String,
        lines: u32,
    },
    Backups,
    /// Summaries the backup scripts leave in /var/lib/backup-all: the R2 copy
    /// (`r2-status.json`) and the last restore test (`restore-test.json`).
    BackupStatus,
    /// Replaces a timer's OnCalendar through a drop-in; `None` stops and disables it.
    /// Only timers whose service is in STYK_AGENT_UNITS.
    TimerSchedule {
        timer: String,
        on_calendar: Option<String>,
    },

    // Self-update through the `styk` CLI (install.sh puts it in /usr/local/bin).
    /// Starts `styk update <version>` as its own systemd unit, so it survives the agent
    /// and the panel being restarted by it. `version` is `latest` or `vX.Y.Z`.
    Update {
        version: String,
    },
    /// Agent version and the last update: /var/lib/styk/update.json plus the log tail.
    UpdateStatus,

    // IP access rules (nft table inet styk_access).
    AccessGet,
    AccessSet {
        config: AccessConfig,
        /// Apply even when no current SSH session would stay allowed.
        #[serde(default)]
        force: bool,
    },

    // Files, sandboxed to the agent's configured roots.
    FsList {
        path: String,
    },
    FsStat {
        path: String,
    },
    FsRead {
        path: String,
        offset: u64,
        len: u32,
    },
    FsWrite {
        path: String,
        content_b64: String,
        in_place: bool,
        expected_mtime: Option<i64>,
    },
    FsUpload {
        path: String,
        offset: u64,
        data_b64: String,
        last: bool,
    },
    FsMkdir {
        path: String,
    },
    FsRename {
        from: String,
        to: String,
    },
    FsTrash {
        path: String,
    },
    FsHistory {
        path: String,
    },

    // Long-running work: start returns an id, poll returns new output lines.
    TaskStart {
        task: Task,
    },
    TaskPoll {
        id: u64,
        offset: usize,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Task {
    /// Only units listed in STYK_AGENT_UNITS.
    RunUnit {
        unit: String,
    },
    PgDump {
        container: String,
        database: String,
        user: String,
        /// The panel's own daily dump: tagged `daily`, older daily dumps of the same
        /// database (beyond 14 days) are removed first.
        #[serde(default)]
        daily: bool,
    },
    /// Dumps the current state first, then restores `file` (must live under /var/backups).
    PgRestore {
        container: String,
        database: String,
        user: String,
        file: String,
    },
    AptUpgrade,
    Compose {
        project: String,
        working_dir: String,
        config_files: Vec<String>,
        action: ComposeAction,
        services: Vec<String>,
    },
    DockerPrune {
        build_cache: bool,
        images: PruneImages,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComposeAction {
    Up,
    Pull,
    /// pull + up -d
    Redeploy,
    Restart,
    Stop,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PruneImages {
    None,
    Dangling,
    Unused,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reply {
    pub id: u64,
    #[serde(flatten)]
    pub result: ReplyResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyResult {
    Ok(serde_json::Value),
    Err(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pong {
    pub version: String,
    pub hostname: String,
}

/// Raw counters; the panel turns consecutive snapshots into rates.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HostSnapshot {
    pub ts_ms: u64,
    pub hostname: String,
    pub kernel: String,
    pub os: String,
    pub uptime_secs: u64,
    /// Index 0 is the aggregate "cpu" line, then one entry per core.
    pub cpu: Vec<CpuTimes>,
    pub load: [f64; 3],
    pub mem: MemInfo,
    pub filesystems: Vec<FsUsage>,
    pub disk_io: IoCounters,
    pub net: NetCounters,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct CpuTimes {
    pub busy: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemInfo {
    pub total: u64,
    pub available: u64,
    pub swap_total: u64,
    pub swap_free: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FsUsage {
    pub mount: String,
    pub device: String,
    pub fstype: String,
    pub total: u64,
    pub used: u64,
    pub inodes_total: u64,
    pub inodes_used: u64,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct IoCounters {
    pub read_bytes: u64,
    pub write_bytes: u64,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct NetCounters {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContainerCgroup {
    pub id: String,
    pub cpu_usage_usec: u64,
    pub mem_current: u64,
    /// `None` when the cgroup has no memory limit.
    pub mem_max: Option<u64>,
    pub io: IoCounters,
    pub pids: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskStarted {
    pub id: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskOutput {
    pub lines: Vec<String>,
    /// Offset to pass to the next poll.
    pub next: usize,
    pub done: bool,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SshActivity {
    pub hours: u32,
    pub failed: u64,
    pub unique_ips: u64,
    /// (unix hour start, failed attempts)
    pub hourly: Vec<(i64, u64)>,
    pub top_ips: Vec<SshIp>,
    pub top_users: Vec<(String, u64)>,
    pub accepted: Vec<SshLogin>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SshIp {
    pub ip: String,
    pub count: u64,
    pub last_user: String,
    pub first: i64,
    pub last: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SshLogin {
    pub ts: i64,
    pub user: String,
    pub ip: String,
    pub method: String,
    /// Key type and fingerprint for publickey logins, e.g. `ED25519 SHA256:…`.
    #[serde(default)]
    pub key: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListeningSocket {
    pub proto: String,
    pub address: String,
    pub port: u16,
    pub process: String,
    pub public: bool,
}

/// One address or network, e.g. `203.0.113.7` or `2001:db8::/32`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AccessEntry {
    pub cidr: String,
    #[serde(default)]
    pub comment: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AccessConfig {
    /// When on, only `ssh_allow` may open port 22.
    pub ssh_allow_enabled: bool,
    pub ssh_allow: Vec<AccessEntry>,
    /// Also allow GitHub Actions runners (CI deploys over SSH). The ranges come from
    /// api.github.com/meta, refreshed daily by styk-access-github-refresh.
    #[serde(default)]
    pub ssh_allow_github: bool,
    /// Dropped on every port before Docker's DNAT.
    pub block: Vec<AccessEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccessState {
    pub config: AccessConfig,
    pub ssh_dropped: u64,
    pub block_dropped: u64,
    /// Peers of the SSH sessions open right now.
    pub ssh_sessions: Vec<String>,
    /// GitHub Actions ranges on disk and when they were last refreshed (unix seconds).
    #[serde(default)]
    pub github_ranges: usize,
    #[serde(default)]
    pub github_updated: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FirewallTable {
    pub family: String,
    pub name: String,
    pub body: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecurityTools {
    pub fail2ban: bool,
    pub crowdsec: bool,
    pub ufw: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Updates {
    pub packages: Vec<UpgradablePackage>,
    pub reboot_required: bool,
    pub reboot_packages: Vec<String>,
    /// Unix seconds of the last `apt update`, if known.
    pub lists_updated: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpgradablePackage {
    pub name: String,
    pub suite: String,
    pub current: String,
    pub candidate: String,
    pub security: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Timer {
    pub unit: String,
    pub activates: String,
    /// Unix microseconds, None when not scheduled / never ran.
    pub next: Option<u64>,
    pub last: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackupFile {
    pub path: String,
    pub size: u64,
    pub mtime: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FsEntry {
    pub name: String,
    pub path: String,
    /// "file" | "dir" | "symlink" | "other"
    pub kind: String,
    pub size: u64,
    pub mtime: i64,
    pub mode: String,
    pub uid: u32,
    pub gid: u32,
    pub writable: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FsListing {
    pub path: String,
    pub root: String,
    pub writable: bool,
    pub entries: Vec<FsEntry>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FsChunk {
    pub data_b64: String,
    pub size: u64,
    pub eof: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FsVersion {
    pub path: String,
    pub ts: i64,
    pub size: u64,
}
