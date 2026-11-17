// Mirrors the JSON returned by styk-panel (crates/styk-panel/src/api).

export interface Me {
  id: number;
  username: string;
  method: "password" | "telegram";
  sudo_until: string | null;
}

export interface FsUsage {
  mount: string;
  device: string;
  fstype: string;
  total: number;
  used: number;
  inodes_total: number;
  inodes_used: number;
}

export interface ContainerSample {
  id: string;
  name: string;
  project: string | null;
  cpu_pct: number;
  mem_used: number;
  mem_limit: number | null;
  pids: number;
}

export interface Sample {
  ts: number;
  cpu_pct: number;
  cores: number[];
  mem_used: number;
  mem_total: number;
  swap_used: number;
  swap_total: number;
  load: [number, number, number];
  disk_read_bps: number;
  disk_write_bps: number;
  net_rx_bps: number;
  net_tx_bps: number;
  filesystems: FsUsage[];
  containers: ContainerSample[];
}

export interface HostInfo {
  hostname: string;
  os: string;
  kernel: string;
  cores: number;
  uptime_secs: number;
}

export interface DockerDiskUsage {
  images_total: number;
  images_count: number;
  images_reclaimable: number;
  build_cache_total: number;
  build_cache_reclaimable: number;
  volumes_total: number;
  volumes_count: number;
  containers_total: number;
}

export interface Overview {
  info: HostInfo | null;
  sample: Sample | null;
  agent_error: string | null;
  docker: DockerDiskUsage | null;
}

export interface HistoryPoint {
  ts: string;
  cpu_pct: number;
  mem_used: number;
  mem_total: number;
  load1: number;
  disk_used: number;
  disk_read_bps: number;
  disk_write_bps: number;
  net_rx_bps: number;
  net_tx_bps: number;
}

export type Range = "1h" | "6h" | "24h" | "7d" | "30d" | "90d";

export interface Container {
  id: string;
  name: string;
  service: string | null;
  image: string;
  state: string;
  status: string;
  health: "healthy" | "unhealthy" | "starting" | null;
  created: number;
  ports: string[];
}

export interface Project {
  name: string;
  working_dir: string | null;
  config_files: string[];
  protected: boolean;
  status: "running" | "partial" | "stopped";
  containers: Container[];
}

export interface ProjectsResp {
  projects: Project[];
  standalone: Container[];
}

export interface EnvVar {
  key: string;
  value: string | null;
  masked: boolean;
  secret: boolean;
}

export interface ContainerInspect {
  id: string;
  name: string;
  image: string;
  created: string;
  restart_count: number;
  state: {
    status: string;
    running: boolean;
    started_at: string;
    finished_at: string;
    exit_code: number;
    oom_killed: boolean;
    health: string | null;
    health_log: { exit_code: number; output: string; end: string }[];
  };
  labels: Record<string, string> | null;
  env: EnvVar[];
  cmd: string[] | null;
  entrypoint: string[] | null;
  restart_policy: string | null;
  memory_limit: number | null;
  nano_cpus: number | null;
  mounts: { type: string; source: string; destination: string; rw: boolean }[];
  networks: string[] | null;
}

export interface LogLine {
  s: "out" | "err" | "sys";
  t: string;
}

export interface AuditRow {
  id: number;
  ts: string;
  username: string | null;
  action: string;
  target: string | null;
  ip: string | null;
  ok: boolean;
  details: Record<string, unknown>;
}

export interface SessionRow {
  method: string;
  ip: string | null;
  user_agent: string | null;
  created_at: string;
  last_seen: string;
  current: boolean;
}

export interface JobRow {
  id: number;
  kind: string;
  target: string;
  status: "running" | "ok" | "failed" | "lost";
  username: string | null;
  exit_code: number | null;
  started_at: string;
  finished_at: string | null;
}

export interface JobTail extends JobRow {
  lines: string[];
  next: number;
}

export interface Finding {
  level: "critical" | "warning" | "ok";
  title: string;
  detail: string;
  fix: string | null;
}

export interface ListeningSocket {
  proto: string;
  address: string;
  port: number;
  process: string;
  public: boolean;
}

export interface UpgradablePackage {
  name: string;
  suite: string;
  current: string;
  candidate: string;
  security: boolean;
}

export interface Updates {
  packages: UpgradablePackage[];
  reboot_required: boolean;
  reboot_packages: string[];
  lists_updated: number | null;
}

export interface SecurityOverview {
  findings: Finding[];
  sshd: Record<string, string>;
  tools: { fail2ban: boolean; crowdsec: boolean; ufw: string | null };
  ports: ListeningSocket[];
  updates: Updates | null;
  errors: Record<string, string>;
}

export interface SshActivity {
  hours: number;
  failed: number;
  unique_ips: number;
  hourly: [number, number][];
  top_ips: { ip: string; count: number; last_user: string; first: number; last: number }[];
  top_users: [string, number][];
  accepted: { ts: number; user: string; ip: string; method: string; key: string }[];
}

export interface FirewallTable {
  family: string;
  name: string;
  body: string;
}

export interface Timer {
  unit: string;
  activates: string;
  next: number | null;
  last: number | null;
}

export interface BackupFile {
  path: string;
  size: number;
  mtime: number;
}

export interface BackupsOverview {
  unit: Record<string, string> | null;
  timer: Timer | null;
  backups: { dirs: string[]; files: BackupFile[] };
  r2: R2Status | null;
  restore_test: {
    unit: Record<string, string> | null;
    timer: Timer | null;
    schedule: RestoreSchedule;
    last: RestoreTest | null;
  };
}

/** Written by backup-r2 after every nightly copy. */
export interface R2Status {
  ts: number;
  snapshots: number;
  latest: { id: string; ts: number; size: number | null } | null;
  stored_bytes: number | null;
  repository: string;
}

export interface RestoreSchedule {
  enabled: boolean;
  days: string[];
  /** HH:MM, UTC */
  time: string;
}

export interface RestoreTest {
  ts: number;
  ok: boolean;
  duration_s: number;
  snapshot: { id: string; ts: number } | null;
  databases: { container: string; dump: string; size: number; image: string; ok: boolean; tables: number; rows: number; error: string | null }[];
  files: { env_files?: number; vaultwarden?: boolean | null };
  error: string | null;
}

export interface PgServer {
  container: string;
  project: string | null;
  image: string;
  user: string;
  default_db: string;
  version: string | null;
  connections: number | null;
  databases: { name: string; size: number; connections: number }[];
  error: string | null;
}

export interface RedisServer {
  container: string;
  project: string | null;
  image: string;
  info: Record<string, string>;
  error: string | null;
}

export interface QueryResult {
  columns?: string[];
  rows?: string[][];
  truncated?: boolean;
  ms?: number;
  notice?: string;
  error?: string;
}

export interface FsEntry {
  name: string;
  path: string;
  kind: "file" | "dir" | "symlink" | "other";
  size: number;
  mtime: number;
  mode: string;
  uid: number;
  gid: number;
  writable: boolean;
}

export interface FsListing {
  path: string;
  root: string;
  writable: boolean;
  entries: FsEntry[];
  truncated: boolean;
}

export interface FsVersion {
  path: string;
  ts: number;
  size: number;
}

export interface FileContent {
  entry: FsEntry;
  binary?: boolean;
  too_large?: boolean;
  content?: string | null;
}

export interface CertInfo {
  domain: string;
  issuer: string;
  not_after: number;
}

export interface CaddySite {
  hosts: string[];
  raw_hosts: string[];
  upstreams: string[];
  cloudflare_only: boolean;
  tls: "http" | "dns" | "on_demand" | "internal";
  redirect: string | null;
  static_files: boolean;
  managed: boolean;
  comment: string;
  line: number;
}

export interface DomainRow {
  site: CaddySite;
  /** matches: how many containers answer to this name on Caddy's networks (null for IPs/FQDNs). */
  upstreams: { address: string; container: string | null; project: string | null; state: string | null; health: string | null; matches: number | null }[];
  certs: CertInfo[];
  uptime: { url: string; up: boolean | null; status: number | null; latency_ms: number | null }[];
}

export interface ProxyProblem {
  kind: "stale" | "missing" | "ambiguous" | "datastore";
  subject: string;
  message: string;
}

export interface DomainsResp {
  sites: DomainRow[];
  problems: ProxyProblem[];
  certs: CertInfo[];
  file: string;
  mtime: number;
  caddy: { container: string; state: string | null; networks: string[] };
  cloudflare_dns: boolean;
}

export interface ApplyResult {
  ok: boolean;
  steps: string[];
  error?: string;
}

export interface RuleDef {
  key: string;
  group: string;
  title: string;
  description: string;
  severity: "info" | "warning" | "critical";
  kind: "state" | "event";
  defaults: Record<string, unknown>;
}

export interface AlertRule {
  def: RuleDef;
  enabled: boolean;
  params: Record<string, unknown>;
  muted_until: string | null;
}

export interface AlertEvent {
  id: number;
  rule: string;
  subject: string;
  severity: "info" | "warning" | "critical";
  status: "firing" | "resolved";
  message: string;
  started_at: string;
  resolved_at: string | null;
  acked_by: string | null;
  acked_at: string | null;
  muted_until: string | null;
  last_notified_at: string | null;
}

export interface AlertPrefs {
  quiet_enabled: boolean;
  quiet_from: number;
  quiet_to: number;
  tz_offset_min: number;
  summary_enabled: boolean;
  summary_hour: number;
  repeat_hours: number;
}

export interface AlertsResp {
  active: AlertEvent[];
  history: AlertEvent[];
  rules: AlertRule[];
  prefs: AlertPrefs;
  telegram: {
    configured: boolean;
    bot_username: string | null;
    chat_id: number | null;
    last_start: { chat_id: number; from_id: number; name: string; linked: boolean; ts: number } | null;
  };
  heartbeat: boolean;
}

export interface UptimeCheck {
  id: number;
  url: string;
  name: string | null;
  source: "caddy" | "manual";
  enabled: boolean;
  up: boolean | null;
  status: number | null;
  latency_ms: number | null;
  error: string | null;
  last_ts: string | null;
  uptime_24h: number | null;
  uptime_7d: number | null;
  avg_latency_24h: number | null;
  recent: boolean[] | null;
}

export interface DeployHook {
  project: string;
  created_by: string | null;
  created_at: string;
  last_used_at: string | null;
}

export interface RetentionPolicy {
  enabled: boolean;
  keep: number;
  protected_tags: string[];
}

export interface RetentionRef {
  reference: string;
  id: string;
  created: number;
  size: number;
  reason: "in_use" | "protected" | "newest" | "";
}

export interface RetentionResp {
  policy: RetentionPolicy;
  plan: { repos: { repo: string; keep: RetentionRef[]; remove: RetentionRef[] }[]; remove_count: number; reclaim: number };
  last: { ts: number; trigger: "manual" | "timer"; removed: number; freed: number; errors: string[] } | null;
  schedule: string;
}

export interface AccessEntry {
  cidr: string;
  comment: string;
}

export interface AccessConfig {
  ssh_allow_enabled: boolean;
  ssh_allow: AccessEntry[];
  ssh_allow_github: boolean;
  block: AccessEntry[];
}

export interface AccessState {
  config: AccessConfig;
  ssh_dropped: number;
  block_dropped: number;
  ssh_sessions: string[];
  github_ranges: number;
  github_updated: number | null;
}

export interface Release {
  version: string;
  url: string;
  published_at: string;
  notes: string;
}

export interface UpdateStatus {
  agent_version: string;
  cli: boolean;
  running: boolean;
  last: { state: "running" | "ok" | "failed"; from: string; to: string; ts: number; error: string } | null;
  log: string;
}

export interface VersionInfo {
  panel: string;
  repo: string;
  latest: Release | null;
  update_available: boolean;
  update: UpdateStatus | null;
}
