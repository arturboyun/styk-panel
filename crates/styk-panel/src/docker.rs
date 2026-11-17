//! Read-only view of Docker: compose projects, containers, disk usage.

use std::collections::BTreeMap;

use bollard::container::ListContainersOptions;
use bollard::models::ContainerSummary;
use serde::Serialize;

use crate::error::ApiResult;
use crate::state::AppState;

const L_PROJECT: &str = "com.docker.compose.project";
const L_SERVICE: &str = "com.docker.compose.service";
const L_WORKDIR: &str = "com.docker.compose.project.working_dir";
const L_CONFIG: &str = "com.docker.compose.project.config_files";

#[derive(Debug, Serialize)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub service: Option<String>,
    pub image: String,
    pub state: String,
    pub status: String,
    pub health: Option<String>,
    pub created: i64,
    pub ports: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Project {
    pub name: String,
    pub working_dir: Option<String>,
    pub config_files: Vec<String>,
    pub protected: bool,
    /// running / partial / stopped
    pub status: &'static str,
    pub containers: Vec<Container>,
}

fn health_of(status: &str) -> Option<String> {
    ["(healthy)", "(unhealthy)", "(health: starting)"]
        .iter()
        .find(|h| status.contains(*h))
        .map(|h| {
            h.trim_matches(|c| c == '(' || c == ')')
                .trim_start_matches("health: ")
                .to_string()
        })
}

fn to_container(c: &ContainerSummary) -> Container {
    let labels = c.labels.clone().unwrap_or_default();
    let status = c.status.clone().unwrap_or_default();
    Container {
        id: c.id.clone().unwrap_or_default(),
        name: c
            .names
            .as_ref()
            .and_then(|n| n.first())
            .map(|n| n.trim_start_matches('/').to_string())
            .unwrap_or_default(),
        service: labels.get(L_SERVICE).cloned(),
        image: c.image.clone().unwrap_or_default(),
        state: c.state.clone().unwrap_or_default(),
        health: health_of(&status),
        status,
        created: c.created.unwrap_or(0),
        ports: c
            .ports
            .iter()
            .flatten()
            .filter_map(|p| {
                let public = p.public_port?;
                let ip = p.ip.as_deref().unwrap_or("");
                Some(format!("{ip}:{public}->{}", p.private_port))
            })
            .collect(),
    }
}

pub async fn projects(state: &AppState) -> ApiResult<(Vec<Project>, Vec<Container>)> {
    let list = state
        .docker
        .list_containers(Some(ListContainersOptions::<String> {
            all: true,
            ..Default::default()
        }))
        .await?;
    let mut by_project: BTreeMap<String, Vec<&ContainerSummary>> = BTreeMap::new();
    let mut standalone = Vec::new();
    for c in &list {
        match c.labels.as_ref().and_then(|l| l.get(L_PROJECT)) {
            Some(p) => by_project.entry(p.clone()).or_default().push(c),
            None => standalone.push(to_container(c)),
        }
    }
    let projects = by_project
        .into_iter()
        .map(|(name, cs)| {
            let labels = cs[0].labels.clone().unwrap_or_default();
            let running = cs
                .iter()
                .filter(|c| c.state.as_deref() == Some("running"))
                .count();
            let mut containers: Vec<Container> = cs.iter().map(|c| to_container(c)).collect();
            containers.sort_by(|a, b| a.name.cmp(&b.name));
            Project {
                protected: state.cfg.protected_projects.contains(&name),
                status: match running {
                    0 => "stopped",
                    n if n == cs.len() => "running",
                    _ => "partial",
                },
                working_dir: labels.get(L_WORKDIR).cloned(),
                config_files: labels
                    .get(L_CONFIG)
                    .map(|s| s.split(',').map(Into::into).collect())
                    .unwrap_or_default(),
                name,
                containers,
            }
        })
        .collect();
    Ok((projects, standalone))
}

#[derive(Debug, Serialize, serde::Deserialize)]
pub struct DiskUsage {
    pub images_total: i64,
    pub images_count: usize,
    pub images_reclaimable: i64,
    pub build_cache_total: i64,
    pub build_cache_reclaimable: i64,
    pub volumes_total: i64,
    pub volumes_count: usize,
    pub containers_total: i64,
}

/// `docker system df` walks every layer and volume, so it is the slowest Docker call; one
/// result serves everyone for a minute. Prunes invalidate `docker:df`.
pub async fn disk_usage(state: &AppState) -> ApiResult<DiskUsage> {
    let v = state
        .cache
        .get_or("docker:df", std::time::Duration::from_secs(60), || async {
            disk_usage_now(state)
                .await
                .map(|d| serde_json::to_value(d).unwrap_or_default())
        })
        .await?;
    Ok(serde_json::from_value(v).map_err(anyhow::Error::from)?)
}

async fn disk_usage_now(state: &AppState) -> ApiResult<DiskUsage> {
    let df = state.docker.df().await?;
    let images = df.images.unwrap_or_default();
    let cache = df.build_cache.unwrap_or_default();
    let volumes = df.volumes.unwrap_or_default();
    Ok(DiskUsage {
        images_total: images.iter().map(|i| i.size).sum(),
        images_count: images.len(),
        images_reclaimable: images
            .iter()
            .filter(|i| i.containers == 0)
            .map(|i| i.size)
            .sum(),
        build_cache_total: cache.iter().filter_map(|c| c.size).sum(),
        build_cache_reclaimable: cache
            .iter()
            .filter(|c| c.in_use != Some(true))
            .filter_map(|c| c.size)
            .sum(),
        volumes_total: volumes
            .iter()
            .filter_map(|v| v.usage_data.as_ref())
            .map(|u| u.size.max(0))
            .sum(),
        volumes_count: volumes.len(),
        containers_total: df
            .containers
            .unwrap_or_default()
            .iter()
            .filter_map(|c| c.size_rw)
            .sum(),
    })
}

pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub code: i64,
}

/// Runs a command inside a container (argv, no shell) and collects its output.
pub async fn exec(
    state: &AppState,
    container: &str,
    cmd: &[&str],
    env: &[&str],
    timeout: std::time::Duration,
) -> anyhow::Result<ExecOutput> {
    use bollard::exec::{CreateExecOptions, StartExecResults};
    use futures::StreamExt;

    let created = state
        .docker
        .create_exec(
            container,
            CreateExecOptions {
                cmd: Some(cmd.iter().map(|s| s.to_string()).collect()),
                env: Some(env.iter().map(|s| s.to_string()).collect()),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                ..Default::default()
            },
        )
        .await?;
    let run = async {
        let mut out = ExecOutput {
            stdout: String::new(),
            stderr: String::new(),
            code: -1,
        };
        if let StartExecResults::Attached { mut output, .. } =
            state.docker.start_exec(&created.id, None).await?
        {
            while let Some(chunk) = output.next().await {
                match chunk? {
                    bollard::container::LogOutput::StdErr { message } => {
                        out.stderr.push_str(&String::from_utf8_lossy(&message))
                    }
                    other => out
                        .stdout
                        .push_str(&String::from_utf8_lossy(&other.into_bytes())),
                }
                if out.stdout.len() > 8 << 20 {
                    break;
                }
            }
        }
        let info = state.docker.inspect_exec(&created.id).await?;
        out.code = info.exit_code.unwrap_or(-1);
        anyhow::Ok(out)
    };
    tokio::time::timeout(timeout, run)
        .await
        .map_err(|_| anyhow::anyhow!("command timed out after {timeout:?}"))?
}

/// Environment of a container as a map (for POSTGRES_USER and friends).
pub async fn container_env(
    state: &AppState,
    container: &str,
) -> anyhow::Result<std::collections::HashMap<String, String>> {
    let info = state
        .docker
        .inspect_container(
            container,
            None::<bollard::container::InspectContainerOptions>,
        )
        .await?;
    Ok(info
        .config
        .and_then(|c| c.env)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|e| {
            e.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect())
}

/// Like `exec`, with `input` written to the command's stdin (then closed).
pub async fn exec_stdin(
    state: &AppState,
    container: &str,
    cmd: &[&str],
    input: &[u8],
    timeout: std::time::Duration,
) -> anyhow::Result<ExecOutput> {
    use bollard::exec::{CreateExecOptions, StartExecResults};
    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;

    let created = state
        .docker
        .create_exec(
            container,
            CreateExecOptions {
                cmd: Some(cmd.iter().map(|s| s.to_string()).collect()),
                attach_stdin: Some(true),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                ..Default::default()
            },
        )
        .await?;
    let run = async {
        let mut out = ExecOutput {
            stdout: String::new(),
            stderr: String::new(),
            code: -1,
        };
        if let StartExecResults::Attached {
            mut output,
            input: mut stdin,
        } = state.docker.start_exec(&created.id, None).await?
        {
            stdin.write_all(input).await?;
            stdin.shutdown().await?;
            drop(stdin);
            while let Some(chunk) = output.next().await {
                match chunk? {
                    bollard::container::LogOutput::StdErr { message } => {
                        out.stderr.push_str(&String::from_utf8_lossy(&message))
                    }
                    other => out
                        .stdout
                        .push_str(&String::from_utf8_lossy(&other.into_bytes())),
                }
            }
        }
        out.code = state
            .docker
            .inspect_exec(&created.id)
            .await?
            .exit_code
            .unwrap_or(-1);
        anyhow::Ok(out)
    };
    tokio::time::timeout(timeout, run)
        .await
        .map_err(|_| anyhow::anyhow!("command timed out after {timeout:?}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_from_status() {
        assert_eq!(
            health_of("Up 3 hours (healthy)").as_deref(),
            Some("healthy")
        );
        assert_eq!(
            health_of("Up 1 second (health: starting)").as_deref(),
            Some("starting")
        );
        assert_eq!(health_of("Up 7 weeks"), None);
    }
}
