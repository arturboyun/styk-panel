//! Long-running work started by the panel.
//!
//! Each task is a list of steps; every step runs as a transient systemd unit
//! (`systemd-run --wait`), so it is outside the agent's cgroup and sandbox and keeps
//! running if the agent or the panel restarts. Output is followed through the journal
//! and buffered here for `TaskPoll`.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use styk_proto::{ComposeAction, PruneImages, Task, TaskOutput, TaskStarted};

use crate::util::{check_name, check_unit, env_list, now_secs, DEFAULT_UNITS};

const MAX_LINES: usize = 50_000;
const POLL_LINES: usize = 2_000;
const KEEP_DONE: Duration = Duration::from_secs(3600);
pub const DUMP_DIR: &str = "/var/backups/styk-panel";

#[derive(Default)]
struct State {
    lines: Vec<String>,
    dropped: bool,
    done: bool,
    exit_code: Option<i32>,
    finished: Option<Instant>,
}

impl State {
    fn push(&mut self, line: String) {
        if self.lines.len() < MAX_LINES {
            self.lines.push(line);
        } else if !self.dropped {
            self.dropped = true;
            self.lines.push("… output truncated".into());
        }
    }
}

struct Entry {
    key: String,
    state: Arc<Mutex<State>>,
}

fn registry() -> &'static Mutex<HashMap<u64, Entry>> {
    static R: OnceLock<Mutex<HashMap<u64, Entry>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

// Ids keep growing across agent restarts, so a stale id is never reused.
static NEXT: AtomicU64 = AtomicU64::new(0);

fn next_id() -> u64 {
    let base = now_secs() as u64 * 1000;
    let _ = NEXT.compare_exchange(0, base, Ordering::SeqCst, Ordering::SeqCst);
    NEXT.fetch_add(1, Ordering::SeqCst)
}

enum Step {
    /// `systemctl start` of an existing unit, output followed via its journal.
    Unit(String),
    Run(Run),
}

#[derive(Default)]
struct Run {
    title: String,
    argv: Vec<String>,
    workdir: Option<String>,
    env: Vec<(String, String)>,
    stdin: Option<String>,
    stdout: Option<String>,
}

impl Run {
    fn new(title: impl Into<String>, argv: &[&str]) -> Self {
        Run {
            title: title.into(),
            argv: argv.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }
}

pub fn start(task: Task) -> Result<TaskStarted> {
    let (key, steps) = plan(&task)?;
    let mut reg = registry().lock().unwrap();
    reg.retain(|_, e| {
        let s = e.state.lock().unwrap();
        !s.finished.is_some_and(|f| f.elapsed() > KEEP_DONE)
    });
    if reg
        .values()
        .any(|e| e.key == key && !e.state.lock().unwrap().done)
    {
        bail!("another task for {key} is still running");
    }
    let id = next_id();
    let state = Arc::new(Mutex::new(State::default()));
    reg.insert(
        id,
        Entry {
            key,
            state: state.clone(),
        },
    );
    drop(reg);
    std::thread::spawn(move || execute(id, steps, state));
    Ok(TaskStarted { id })
}

pub fn poll(id: u64, offset: usize) -> Result<TaskOutput> {
    let reg = registry().lock().unwrap();
    let e = reg
        .get(&id)
        .context("unknown task (the agent may have restarted)")?;
    let s = e.state.lock().unwrap();
    let from = offset.min(s.lines.len());
    let to = (from + POLL_LINES).min(s.lines.len());
    Ok(TaskOutput {
        lines: s.lines[from..to].to_vec(),
        next: to,
        done: s.done && to == s.lines.len(),
        exit_code: s.exit_code,
    })
}

fn execute(id: u64, steps: Vec<Step>, state: Arc<Mutex<State>>) {
    let mut code = 0;
    let n = steps.len();
    for (i, step) in steps.into_iter().enumerate() {
        let unit = format!("styk-task-{id}-{i}");
        let title = match &step {
            Step::Unit(u) => format!("systemctl start {u}"),
            Step::Run(r) => r.title.clone(),
        };
        state
            .lock()
            .unwrap()
            .push(format!("▶ [{}/{n}] {title}", i + 1));
        let res = match step {
            Step::Unit(u) => run_unit(&u, &state),
            Step::Run(r) => run_transient(&unit, r, &state),
        };
        code = match res {
            Ok(c) => c,
            Err(e) => {
                state.lock().unwrap().push(format!("error: {e:#}"));
                -1
            }
        };
        if code != 0 {
            state
                .lock()
                .unwrap()
                .push(format!("✗ step failed with exit code {code}"));
            break;
        }
    }
    let mut s = state.lock().unwrap();
    if code == 0 {
        s.push("✓ done".into());
    }
    s.done = true;
    s.exit_code = Some(code);
    s.finished = Some(Instant::now());
}

fn follow(unit: &str, state: &Arc<Mutex<State>>) -> Result<(Child, std::thread::JoinHandle<()>)> {
    let mut child = Command::new("journalctl")
        .args(["-f", "-n0", "-o", "cat", "--no-pager", "-u", unit])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("journalctl")?;
    let out = child.stdout.take().context("journalctl stdout")?;
    let st = state.clone();
    let h = std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            st.lock().unwrap().push(line);
        }
    });
    // Let journalctl attach before the unit starts logging.
    std::thread::sleep(Duration::from_millis(300));
    Ok((child, h))
}

fn finish_follow(mut child: Child, h: std::thread::JoinHandle<()>) {
    std::thread::sleep(Duration::from_millis(1200));
    let _ = child.kill();
    let _ = child.wait();
    let _ = h.join();
}

fn waited(mut cmd: Command, state: &Arc<Mutex<State>>) -> Result<i32> {
    let out = cmd.stdin(Stdio::null()).output()?;
    for line in String::from_utf8_lossy(&out.stderr).lines() {
        state.lock().unwrap().push(line.to_string());
    }
    Ok(out.status.code().unwrap_or(-1))
}

fn run_unit(unit: &str, state: &Arc<Mutex<State>>) -> Result<i32> {
    let (child, h) = follow(unit, state)?;
    let mut cmd = Command::new("systemctl");
    cmd.args(["start", "--", unit]);
    let code = waited(cmd, state);
    finish_follow(child, h);
    code
}

fn run_transient(unit: &str, r: Run, state: &Arc<Mutex<State>>) -> Result<i32> {
    let mut cmd = Command::new("systemd-run");
    cmd.args(["--quiet", "--wait", "--collect", "--service-type=exec"])
        .arg(format!("--unit={unit}"))
        .arg(format!("--description=styk-panel: {}", r.title));
    if let Some(wd) = &r.workdir {
        cmd.arg(format!("--working-directory={wd}"));
    }
    for (k, v) in &r.env {
        cmd.arg(format!("--setenv={k}={v}"));
    }
    if let Some(p) = &r.stdin {
        cmd.arg(format!("--property=StandardInput=file:{p}"));
    }
    if let Some(p) = &r.stdout {
        cmd.arg(format!("--property=StandardOutput=truncate:{p}"));
        cmd.arg("--property=StandardError=journal");
    }
    cmd.arg("--").args(&r.argv);
    let (child, h) = follow(unit, state)?;
    let code = waited(cmd, state);
    finish_follow(child, h);
    code
}

fn plan(task: &Task) -> Result<(String, Vec<Step>)> {
    Ok(match task {
        Task::RunUnit { unit } => {
            check_unit(unit)?;
            if !env_list("STYK_AGENT_UNITS", DEFAULT_UNITS).contains(unit) {
                bail!("{unit} is not in STYK_AGENT_UNITS");
            }
            (format!("unit:{unit}"), vec![Step::Unit(unit.clone())])
        }
        Task::PgDump {
            container,
            database,
            user,
            daily,
        } => {
            check_pg(container, database, user)?;
            let tag = if *daily { "daily" } else { "manual" };
            if *daily {
                prune_daily(container, database, Duration::from_secs(14 * 86400));
            }
            let out = dump_path(container, database, tag)?;
            (
                format!("pg:{container}"),
                vec![pg_dump(container, database, user, &out)],
            )
        }
        Task::PgRestore {
            container,
            database,
            user,
            file,
        } => {
            check_pg(container, database, user)?;
            let file = Path::new(file)
                .canonicalize()
                .with_context(|| format!("{file}: not found"))?;
            if !file.starts_with("/var/backups") || !file.is_file() {
                bail!("restore source must be a file under /var/backups");
            }
            let file = file.to_string_lossy().into_owned();
            let safety = dump_path(container, database, "pre-restore")?;
            let mut restore = Run::new(
                format!("pg_restore {file} → {container}/{database}"),
                &[
                    "docker",
                    "exec",
                    "-i",
                    container,
                    "pg_restore",
                    "-U",
                    user,
                    "-d",
                    database,
                    "--clean",
                    "--if-exists",
                    "--no-owner",
                    "--exit-on-error",
                ],
            );
            restore.stdin = Some(file);
            (
                format!("pg:{container}"),
                vec![
                    pg_dump(container, database, user, &safety),
                    Step::Run(restore),
                ],
            )
        }
        Task::AptUpgrade => {
            let env = vec![("DEBIAN_FRONTEND".to_string(), "noninteractive".to_string())];
            let mut update = Run::new("apt-get update", &["apt-get", "update"]);
            update.env = env.clone();
            let mut upgrade = Run::new(
                "apt-get upgrade",
                &[
                    "apt-get",
                    "-y",
                    "-o",
                    "Dpkg::Options::=--force-confdef",
                    "-o",
                    "Dpkg::Options::=--force-confold",
                    "--with-new-pkgs",
                    "upgrade",
                ],
            );
            upgrade.env = env;
            ("apt".into(), vec![Step::Run(update), Step::Run(upgrade)])
        }
        Task::Compose {
            project,
            working_dir,
            config_files,
            action,
            services,
        } => {
            check_name("project", project)?;
            for s in services {
                check_name("service", s)?;
            }
            check_protected(project, *action, services)?;
            let wd = Path::new(working_dir)
                .canonicalize()
                .context("working dir")?;
            if !wd.is_dir() || !(wd.starts_with("/opt") || wd.starts_with("/srv")) {
                bail!("compose projects must live under /opt or /srv");
            }
            let mut base = vec![
                "docker".to_string(),
                "compose".into(),
                "-p".into(),
                project.clone(),
                "--project-directory".into(),
                wd.to_string_lossy().into_owned(),
            ];
            for f in config_files {
                let f = Path::new(f)
                    .canonicalize()
                    .with_context(|| format!("{f}: not found"))?;
                if !f.is_file() {
                    bail!("{} is not a file", f.display());
                }
                base.push("-f".into());
                base.push(f.to_string_lossy().into_owned());
            }
            let cmd = |args: &[&str]| -> Step {
                let mut argv = base.clone();
                argv.extend(args.iter().map(|s| s.to_string()));
                argv.extend(services.iter().cloned());
                Step::Run(Run {
                    title: format!("docker compose {}", args.join(" ")),
                    argv,
                    workdir: Some(wd.to_string_lossy().into_owned()),
                    ..Default::default()
                })
            };
            let steps = match action {
                ComposeAction::Up => vec![cmd(&["up", "-d"])],
                ComposeAction::Pull => vec![cmd(&["pull"])],
                ComposeAction::Redeploy => vec![cmd(&["pull"]), cmd(&["up", "-d"])],
                ComposeAction::Restart => vec![cmd(&["restart"])],
                ComposeAction::Stop => vec![cmd(&["stop"])],
                ComposeAction::Down => vec![cmd(&["down"])],
            };
            (format!("compose:{project}"), steps)
        }
        Task::DockerPrune {
            build_cache,
            images,
        } => {
            let mut steps = Vec::new();
            if *build_cache {
                steps.push(Step::Run(Run::new(
                    "docker builder prune",
                    &["docker", "builder", "prune", "-f"],
                )));
            }
            match images {
                PruneImages::None => {}
                PruneImages::Dangling => steps.push(Step::Run(Run::new(
                    "docker image prune",
                    &["docker", "image", "prune", "-f"],
                ))),
                PruneImages::Unused => steps.push(Step::Run(Run::new(
                    "docker image prune -a",
                    &["docker", "image", "prune", "-a", "-f"],
                ))),
            }
            if steps.is_empty() {
                bail!("nothing to prune");
            }
            ("docker-prune".into(), steps)
        }
    })
}

fn check_pg(container: &str, database: &str, user: &str) -> Result<()> {
    check_name("container", container)?;
    check_name("database", database)?;
    check_name("user", user)
}

/// Removes `daily` dumps of one database older than `keep`.
fn prune_daily(container: &str, database: &str, keep: Duration) {
    let prefix = format!("{container}-{database}-daily-");
    let Ok(dir) = std::fs::read_dir(DUMP_DIR) else {
        return;
    };
    for e in dir.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > keep);
        if name.starts_with(&prefix) && name.ends_with(".dump") && old {
            if let Err(err) = std::fs::remove_file(e.path()) {
                tracing::warn!("remove {name}: {err}");
            }
        }
    }
}

fn dump_path(container: &str, database: &str, tag: &str) -> Result<String> {
    std::fs::create_dir_all(DUMP_DIR)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(DUMP_DIR, std::fs::Permissions::from_mode(0o700))?;
    let ts = chrono_like_ts();
    Ok(format!("{DUMP_DIR}/{container}-{database}-{tag}-{ts}.dump"))
}

fn pg_dump(container: &str, database: &str, user: &str, out: &str) -> Step {
    let mut r = Run::new(
        format!("pg_dump {container}/{database} → {out}"),
        &[
            "docker", "exec", container, "pg_dump", "-U", user, "-d", database, "-Fc",
        ],
    );
    r.stdout = Some(out.to_string());
    Step::Run(r)
}

/// `20261103T120000Z`, same shape as backup-all's file names.
fn chrono_like_ts() -> String {
    let secs = now_secs();
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Projects the whole host depends on (the panel and its Caddy, a shared proxy) can only have
/// individual non-proxy services restarted or redeployed.
fn check_protected(project: &str, action: ComposeAction, services: &[String]) -> Result<()> {
    if !env_list("STYK_AGENT_PROTECTED", "styk-panel")
        .iter()
        .any(|p| p == project)
    {
        return Ok(());
    }
    if matches!(action, ComposeAction::Stop | ComposeAction::Down) {
        bail!("{project} is protected: stop/down is not allowed");
    }
    if services.is_empty() {
        bail!("{project} is protected: name the services explicitly");
    }
    if services.iter().any(|s| s.contains("caddy")) {
        bail!("{project} is protected: Caddy is only reloaded, never restarted");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected() {
        let svc = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert!(check_protected("shop", ComposeAction::Down, &[]).is_ok());
        assert!(check_protected("styk-panel", ComposeAction::Down, &svc(&["api"])).is_err());
        assert!(check_protected("styk-panel", ComposeAction::Restart, &[]).is_err());
        assert!(check_protected("styk-panel", ComposeAction::Restart, &svc(&["caddy"])).is_err());
        assert!(check_protected("styk-panel", ComposeAction::Redeploy, &svc(&["api"])).is_ok());
        assert!(check_protected("styk-panel", ComposeAction::Stop, &svc(&["panel"])).is_err());
    }

    #[test]
    fn plans() {
        assert!(plan(&Task::RunUnit {
            unit: "ssh.service".into()
        })
        .is_err());
        assert!(plan(&Task::RunUnit {
            unit: "backup-all.service".into()
        })
        .is_ok());
        let restore = Task::PgRestore {
            container: "db".into(),
            database: "app".into(),
            user: "app".into(),
            file: "/etc/passwd".into(),
        };
        assert!(plan(&restore).is_err());
        let bad = Task::PgDump {
            container: "db".into(),
            database: "-c".into(),
            user: "u".into(),
            daily: false,
        };
        assert!(plan(&bad).is_err());
        let prune = Task::DockerPrune {
            build_cache: false,
            images: PruneImages::None,
        };
        assert!(plan(&prune).is_err());
    }

    #[test]
    fn ts_shape() {
        let t = chrono_like_ts();
        assert_eq!(t.len(), 16);
        assert!(t.starts_with("20"));
        assert!(t.ends_with('Z'));
    }
}
