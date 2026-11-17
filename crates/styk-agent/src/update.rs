//! Self-update: the agent only starts `styk update` (the CLI from install.sh) as its own
//! transient unit and reports what it left in /var/lib/styk. The CLI downloads the release,
//! checks its minisign signature and checksums, swaps the agent and the panel image and
//! rolls back on a failed health check; it restarts this agent, hence the separate unit.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

const CLI: &str = "/usr/local/bin/styk";
const STATE: &str = "/var/lib/styk/update.json";
const LOG: &str = "/var/lib/styk/update.log";
const UNIT: &str = "styk-update.service";

/// `latest` or `vMAJOR.MINOR.PATCH` with an optional `-pre.release` suffix.
fn valid_version(v: &str) -> bool {
    if v == "latest" {
        return true;
    }
    let Some(rest) = v.strip_prefix('v') else {
        return false;
    };
    let (core, pre) = match rest.split_once('-') {
        Some((_, "")) => return false,
        Some((c, p)) => (c, p),
        None => (rest, ""),
    };
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 6 && p.bytes().all(|b| b.is_ascii_digit()))
        && pre.len() <= 32
        && pre.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.')
}

fn running() -> bool {
    Command::new("systemctl")
        .args(["is-active", "--quiet", UNIT])
        .status()
        .is_ok_and(|s| s.success())
}

pub fn start(version: &str) -> Result<Value> {
    if !valid_version(version) {
        bail!("version must be `latest` or vX.Y.Z");
    }
    if !Path::new(CLI).is_file() {
        bail!("{CLI} is missing: this server was not set up with install.sh");
    }
    if running() {
        bail!("an update is already running");
    }
    let out = Command::new("systemd-run")
        .args(["--quiet", "--no-block", "--collect", "--service-type=exec"])
        .arg(format!("--unit={UNIT}"))
        .arg(format!("--description=Styk Panel update to {version}"))
        .args([CLI, "update", "--yes", version])
        .output()
        .context("systemd-run")?;
    if !out.status.success() {
        bail!(
            "systemd-run: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(json!({ "started": true, "version": version }))
}

pub fn status() -> Value {
    let state: Value = std::fs::read_to_string(STATE)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    let log = std::fs::read_to_string(LOG).unwrap_or_default();
    let lines: Vec<&str> = log.lines().collect();
    let tail = lines[lines.len().saturating_sub(200)..].join("\n");
    json!({
        "agent_version": env!("CARGO_PKG_VERSION"),
        "cli": Path::new(CLI).is_file(),
        "running": running(),
        "last": state,
        "log": tail,
    })
}

#[cfg(test)]
mod tests {
    use super::valid_version;

    #[test]
    fn versions() {
        for ok in ["latest", "v0.1.0", "v12.3.45", "v1.0.0-rc.1"] {
            assert!(valid_version(ok), "{ok}");
        }
        for bad in [
            "",
            "v1",
            "1.0.0",
            "v1.0",
            "v1.0.0;rm",
            "v1.0.0-",
            "v1.0.0 x",
            "--yes",
        ] {
            assert!(!valid_version(bad), "{bad}");
        }
    }
}
