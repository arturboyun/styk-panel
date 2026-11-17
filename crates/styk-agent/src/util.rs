//! Small helpers for running host tools and validating names that end up in argv.

use std::process::Command;

use anyhow::{bail, Context, Result};

/// Runs a tool and returns stdout; a non-zero exit is an error carrying stderr.
pub fn run(cmd: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(cmd)
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .with_context(|| format!("run {cmd}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("{cmd} exited with {}: {}", out.status, err.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Like `run`, but a missing tool or a failure yields `None`.
pub fn try_run(cmd: &str, args: &[&str]) -> Option<String> {
    run(cmd, args).ok()
}

/// Container, database, user, project and service names: conservative charset, no
/// leading dash (so they can never be read as an option).
pub fn check_name(kind: &str, s: &str) -> Result<()> {
    let ok = !s.is_empty()
        && s.len() <= 128
        && !s.starts_with('-')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if !ok {
        bail!("invalid {kind}: {s:?}");
    }
    Ok(())
}

/// systemd unit names (allow `@` instances).
pub fn check_unit(s: &str) -> Result<()> {
    let ok = !s.is_empty()
        && s.len() <= 200
        && !s.starts_with('-')
        && s.contains('.')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '@' | ':'));
    if !ok {
        bail!("invalid unit: {s:?}");
    }
    Ok(())
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Units the panel may start (and whose timers it may reschedule) unless STYK_AGENT_UNITS says otherwise.
pub const DEFAULT_UNITS: &str = "backup-all.service,backup-restore-test.service";

pub fn env_list(key: &str, default: &str) -> Vec<String> {
    std::env::var(key)
        .unwrap_or_else(|_| default.into())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(check_name("db", "my_app").is_ok());
        assert!(check_name("db", "-c").is_err());
        assert!(check_name("db", "a b").is_err());
        assert!(check_name("db", "a;rm").is_err());
        assert!(check_unit("backup-all.service").is_ok());
        assert!(check_unit("getty@tty1.service").is_ok());
        assert!(check_unit("backup-all").is_err());
        assert!(check_unit("--all.service").is_err());
    }
}
