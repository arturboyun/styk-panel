//! Domains: sites from the shared Caddyfile with their upstream containers, certificates
//! and uptime; the wizard adds/removes panel-managed blocks; the raw editor saves the
//! whole file. Every change: validate → write in place → reload → roll back on failure.

use std::collections::HashMap;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::Json;
use base64::Engine;
use bollard::container::ListContainersOptions;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use styk_proto::{FsChunk, FsEntry, Method};

use crate::audit;
use crate::auth::{AuthUser, ClientInfo};
use crate::caddyfile::{self, NewSite};
use crate::certs::{self, CertInfo};
use crate::docker::{container_env, exec, exec_stdin};
use crate::error::{agent_error, ApiError, ApiResult};
use crate::state::AppState;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

pub async fn read_caddyfile(st: &AppState) -> ApiResult<(String, i64)> {
    let path = st.cfg.caddyfile.clone();
    let stat: FsEntry = st
        .agent
        .call(Method::FsStat { path: path.clone() })
        .await
        .map_err(agent_error)?;
    let c: FsChunk = st
        .agent
        .call(Method::FsRead {
            path,
            offset: 0,
            len: 1 << 20,
        })
        .await
        .map_err(agent_error)?;
    let bytes = B64.decode(c.data_b64).map_err(anyhow::Error::from)?;
    Ok((String::from_utf8_lossy(&bytes).into_owned(), stat.mtime))
}

async fn write_caddyfile(
    st: &AppState,
    text: &str,
    expected_mtime: Option<i64>,
) -> ApiResult<FsEntry> {
    st.agent
        .call(Method::FsWrite {
            path: st.cfg.caddyfile.clone(),
            content_b64: B64.encode(text.as_bytes()),
            in_place: true,
            expected_mtime,
        })
        .await
        .map_err(agent_error)
}

/// Caddy's environment (placeholders like `{env.X}`); it changes only when the container is
/// recreated, which ends a job and invalidates `docker:`.
pub async fn caddy_env(st: &AppState) -> HashMap<String, String> {
    st.cache
        .get_or(
            "docker:caddy_env",
            std::time::Duration::from_secs(300),
            || async {
                container_env(st, &st.cfg.caddy_container)
                    .await
                    .map(|e| serde_json::to_value(e).unwrap_or_default())
            },
        )
        .await
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

/// Caddy logs JSON lines; the useful part of a failure is the last error message.
fn caddy_error(out: &str) -> String {
    out.lines()
        .rev()
        .find_map(|l| {
            let v: Value = serde_json::from_str(l).ok()?;
            (v["level"] == "error").then(|| {
                let msg = v["msg"].as_str().unwrap_or("");
                match v["error"].as_str() {
                    Some(e) => format!("{msg}: {e}"),
                    None => msg.to_string(),
                }
            })
        })
        .unwrap_or_else(|| out.lines().last().unwrap_or("").to_string())
}

async fn validate(st: &AppState, text: &str) -> Result<(), String> {
    let out = exec_stdin(
        st,
        &st.cfg.caddy_container,
        &[
            "caddy",
            "validate",
            "--config",
            "/dev/stdin",
            "--adapter",
            "caddyfile",
        ],
        text.as_bytes(),
        Duration::from_secs(30),
    )
    .await
    .map_err(|e| e.to_string())?;
    if out.code == 0 {
        Ok(())
    } else {
        Err(caddy_error(&format!("{}{}", out.stdout, out.stderr)))
    }
}

async fn reload(st: &AppState) -> Result<(), String> {
    let out = exec(
        st,
        &st.cfg.caddy_container,
        &[
            "caddy",
            "reload",
            "--config",
            &st.cfg.caddy_config,
            "--adapter",
            "caddyfile",
        ],
        &[],
        Duration::from_secs(60),
    )
    .await
    .map_err(|e| e.to_string())?;
    if out.code == 0 {
        Ok(())
    } else {
        Err(caddy_error(&format!("{}{}", out.stdout, out.stderr)))
    }
}

async fn container_sees(st: &AppState, text: &str) -> bool {
    match exec(
        st,
        &st.cfg.caddy_container,
        &["cat", &st.cfg.caddy_config],
        &[],
        Duration::from_secs(10),
    )
    .await
    {
        Ok(o) => o.code == 0 && o.stdout == text,
        Err(_) => false,
    }
}

/// The one write path for the Caddyfile. Returns the step log.
async fn apply(
    st: &AppState,
    u: &AuthUser,
    client: &ClientInfo,
    action: &str,
    target: &str,
    change: impl FnOnce(&str) -> Result<String, String>,
    expected_mtime: Option<i64>,
) -> ApiResult<Json<Value>> {
    let _lock = st.caddy_lock.lock().await;
    let (old, mtime) = read_caddyfile(st).await?;
    if expected_mtime.is_some_and(|m| m != mtime) {
        return Err(ApiError::BadRequest(
            "Caddyfile изменился с момента открытия, обновите страницу".into(),
        ));
    }
    let new = change(&old).map_err(ApiError::BadRequest)?;
    let mut steps = vec!["Caddyfile прочитан".to_string()];
    let fail = |steps: Vec<String>, err: String| -> ApiResult<Json<Value>> {
        Ok(Json(json!({ "ok": false, "steps": steps, "error": err })))
    };

    let result: Result<(), String> = async {
        validate(st, &new)
            .await
            .map_err(|e| format!("caddy validate: {e}"))?;
        steps.push("caddy validate: ok".into());
        write_caddyfile(st, &new, Some(mtime))
            .await
            .map_err(|e| e.to_string())?;
        steps.push("записан на место (in-place), прошлая версия в истории".into());
        if !container_sees(st, &new).await {
            return Err("контейнер Caddy не видит новый файл (bind mount)".into());
        }
        reload(st).await.map_err(|e| format!("caddy reload: {e}"))?;
        steps.push("caddy reload: ok".into());
        Ok(())
    }
    .await;

    let ok = result.is_ok();
    audit::user_action(
        st,
        u,
        client,
        action,
        target,
        ok,
        json!({ "error": result.as_ref().err() }),
    )
    .await;
    if let Err(err) = result {
        // Only roll back what we actually wrote.
        let (current, _) = read_caddyfile(st).await.unwrap_or_default();
        if current == new {
            match write_caddyfile(st, &old, None).await {
                Ok(_) => {
                    let r = reload(st).await;
                    steps.push(format!(
                        "откат: прежний файл восстановлен{}",
                        if r.is_ok() {
                            ", caddy reload: ok"
                        } else {
                            ", reload не удался"
                        }
                    ));
                }
                Err(e) => steps.push(format!("откат не удался: {e}")),
            }
        }
        return fail(steps, err);
    }
    st.cache.invalidate("domains:");
    Ok(Json(json!({ "ok": true, "steps": steps })))
}

const CERT_SCRIPT: &str = r#"find /data/caddy/certificates -name '*.crt' | while read -r f; do echo "==> $f"; cat "$f"; done"#;

pub async fn certificates(st: &AppState) -> Vec<CertInfo> {
    let v = st
        .cache
        .get_or("domains:certs", Duration::from_secs(600), || async {
            let out = exec(
                st,
                &st.cfg.caddy_container,
                &["sh", "-c", CERT_SCRIPT],
                &[],
                Duration::from_secs(20),
            )
            .await
            .map_err(|e| e.to_string())?;
            Ok::<_, String>(
                serde_json::to_value(certs::parse_dump(&out.stdout)).unwrap_or_default(),
            )
        })
        .await
        .unwrap_or_default();
    serde_json::from_value(v).unwrap_or_default()
}

#[derive(Serialize, Deserialize, Clone)]
struct Endpoint {
    container: String,
    project: Option<String>,
    #[serde(default)]
    image: String,
    state: String,
    health: Option<String>,
    names: Vec<String>,
    /// Names per network: the container name plus that network's aliases. Compose adds the
    /// bare service name (`api`, `web`) on every network, so these collide on shared ones.
    #[serde(default)]
    net_names: HashMap<String, Vec<String>>,
    ports: Vec<u16>,
    networks: Vec<String>,
}

impl Endpoint {
    /// Whether `host` resolves to this container on any of `nets`.
    fn answers_on(&self, host: &str, nets: &[String]) -> bool {
        self.state == "running"
            && nets.iter().any(|n| {
                self.net_names
                    .get(n)
                    .is_some_and(|names| names.iter().any(|x| x == host))
            })
    }
}

/// Running containers with every name they answer to on Docker networks.
async fn endpoints(st: &AppState) -> Vec<Endpoint> {
    let v = st
        .cache
        .get_or("domains:endpoints", Duration::from_secs(30), || async {
            let list = st
                .docker
                .list_containers(Some(ListContainersOptions::<String> {
                    all: true,
                    ..Default::default()
                }))
                .await
                .map_err(|e| e.to_string())?;
            let futs = list
                .iter()
                .filter_map(|c| c.id.clone())
                .map(|id| async move { st.docker.inspect_container(&id, None).await.ok() });
            let infos = futures::future::join_all(futs).await;
            let eps: Vec<Endpoint> = infos
                .into_iter()
                .flatten()
                .map(|i| {
                    let name = i
                        .name
                        .clone()
                        .unwrap_or_default()
                        .trim_start_matches('/')
                        .to_string();
                    let cfg = i.config.clone().unwrap_or_default();
                    let labels = cfg.labels.clone().unwrap_or_default();
                    let mut names = vec![name.clone()];
                    let mut networks = Vec::new();
                    let mut net_names = HashMap::new();
                    if let Some(nets) = i.network_settings.as_ref().and_then(|n| n.networks.clone())
                    {
                        for (net, ep) in nets {
                            let mut here = vec![name.clone()];
                            here.extend(ep.aliases.unwrap_or_default());
                            here.extend(ep.dns_names.unwrap_or_default());
                            here.sort();
                            here.dedup();
                            names.extend(here.iter().cloned());
                            networks.push(net.clone());
                            net_names.insert(net, here);
                        }
                    }
                    if let Some(s) = labels.get("com.docker.compose.service") {
                        names.push(s.clone());
                    }
                    names.sort();
                    names.dedup();
                    let ports = cfg
                        .exposed_ports
                        .unwrap_or_default()
                        .keys()
                        .filter_map(|k| k.split('/').next()?.parse().ok())
                        .collect();
                    let st = i.state.clone().unwrap_or_default();
                    Endpoint {
                        container: name,
                        project: labels.get("com.docker.compose.project").cloned(),
                        image: cfg.image.clone().unwrap_or_default(),
                        net_names,
                        state: st.status.map(|s| s.to_string()).unwrap_or_default(),
                        health: st.health.and_then(|h| h.status).map(|s| s.to_string()),
                        names,
                        ports,
                        networks,
                    }
                })
                .collect();
            Ok::<_, String>(serde_json::to_value(eps).unwrap_or_default())
        })
        .await
        .unwrap_or_default();
    serde_json::from_value(v).unwrap_or_default()
}

fn upstream_host(u: &str) -> &str {
    let u = u
        .strip_prefix("http://")
        .or_else(|| u.strip_prefix("https://"))
        .unwrap_or(u);
    u.rsplit_once(':').map(|(h, _)| h).unwrap_or(u)
}

/// Upstreams that are plain container names (not IPs, FQDNs, localhost or placeholders).
fn container_host(u: &str) -> Option<&str> {
    let h = upstream_host(u);
    let plain = !h.is_empty()
        && h != "localhost"
        && !h.contains(['.', '{', '[', '$'])
        && h.parse::<std::net::IpAddr>().is_err();
    plain.then_some(h)
}

/// Images that hold data and should never sit on the proxy network.
fn is_datastore(image: &str) -> bool {
    let repo = image.rsplit('/').next().unwrap_or(image);
    let name = repo.split([':', '@']).next().unwrap_or(repo);
    [
        "postgres",
        "postgis",
        "mysql",
        "mariadb",
        "mongo",
        "redis",
        "keydb",
        "valkey",
        "memcached",
        "clickhouse",
    ]
    .iter()
    .any(|d| name.starts_with(d))
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ProxyProblem {
    /// "stale" | "missing" | "ambiguous" | "datastore"
    pub kind: &'static str,
    pub subject: String,
    pub message: String,
}

fn network_problems(
    text: &str,
    env: &HashMap<String, String>,
    eps: &[Endpoint],
    caddy: &str,
) -> Vec<ProxyProblem> {
    let nets: Vec<String> = eps
        .iter()
        .find(|e| e.container == caddy)
        .map(|e| e.networks.clone())
        .unwrap_or_default();
    if nets.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for site in caddyfile::sites(&caddyfile::parse(text), env) {
        for u in &site.upstreams {
            let Some(host) = container_host(u) else {
                continue;
            };
            if !seen.insert(host.to_string()) {
                continue;
            }
            let hits: Vec<&str> = eps
                .iter()
                .filter(|e| e.container != caddy && e.answers_on(host, &nets))
                .map(|e| e.container.as_str())
                .collect();
            match hits.len() {
                0 => out.push(ProxyProblem {
                    kind: "missing",
                    subject: host.to_string(),
                    message: format!("{host} ({}) не найден в сети Caddy: сайт отдаёт 502", site.hosts.join(", ")),
                }),
                1 => {}
                _ => out.push(ProxyProblem {
                    kind: "ambiguous",
                    subject: host.to_string(),
                    message: format!(
                        "{host} ({}) отвечает у {} контейнеров: {}. Caddy будет ходить в случайный; дайте сервису уникальный alias",
                        site.hosts.join(", "),
                        hits.len(),
                        hits.join(", ")
                    ),
                }),
            }
        }
    }
    for e in eps
        .iter()
        .filter(|e| e.state == "running" && is_datastore(&e.image))
    {
        if let Some(n) = e.networks.iter().find(|n| nets.contains(n)) {
            out.push(ProxyProblem {
                kind: "datastore",
                subject: e.container.clone(),
                message: format!(
                    "{} ({}) подключён к общей сети {n}: базы и кэши должны быть только во внутренней сети проекта",
                    e.container, e.image
                ),
            });
        }
    }
    out
}

/// Problems with the shared proxy: Caddy reading a stale Caddyfile, upstream names that
/// resolve to no container or to several on Caddy's networks, datastores on those networks.
pub async fn proxy_problems(st: &AppState) -> ApiResult<Vec<ProxyProblem>> {
    let (text, _) = read_caddyfile(st).await?;
    let (env, eps) = tokio::join!(caddy_env(st), endpoints(st));
    let mut out = Vec::new();
    if eps
        .iter()
        .any(|e| e.container == st.cfg.caddy_container && e.state == "running")
        && !container_sees(st, &text).await
    {
        out.push(ProxyProblem {
            kind: "stale",
            subject: st.cfg.caddy_container.clone(),
            message: format!(
                "{} читает не тот Caddyfile, что лежит в {}: правки не применяются. Обычно это монтирование одного файла, который заменили через rename; монтируйте папку",
                st.cfg.caddy_container, st.cfg.caddyfile
            ),
        });
    }
    out.extend(network_problems(&text, &env, &eps, &st.cfg.caddy_container));
    Ok(out)
}

pub async fn list(State(st): State<AppState>, _u: AuthUser) -> ApiResult<Json<Value>> {
    let (text, mtime) = read_caddyfile(&st).await?;
    let (env, certs, eps) = tokio::join!(caddy_env(&st), certificates(&st), endpoints(&st));
    let caddy_networks: Vec<String> = eps
        .iter()
        .find(|e| e.container == st.cfg.caddy_container)
        .map(|e| e.networks.clone())
        .unwrap_or_default();
    let uptime: Vec<(String, Option<bool>, Option<i32>, Option<i32>)> = sqlx::query_as(
        "SELECT c.url, r.up, r.status, r.latency_ms FROM uptime_checks c
           LEFT JOIN LATERAL (SELECT up, status, latency_ms FROM uptime_results WHERE check_id = c.id ORDER BY ts DESC LIMIT 1) r ON true",
    )
    .fetch_all(&st.db)
    .await?;
    let sites: Vec<Value> = caddyfile::sites(&caddyfile::parse(&text), &env)
        .into_iter()
        .map(|s| {
            let upstreams: Vec<Value> = s
                .upstreams
                .iter()
                .map(|u| {
                    let host = upstream_host(u);
                    let hits: Vec<&Endpoint> = eps
                        .iter()
                        .filter(|e| e.container != st.cfg.caddy_container)
                        .filter(|e| {
                            if caddy_networks.is_empty() {
                                e.names.iter().any(|n| n == host)
                            } else {
                                e.answers_on(host, &caddy_networks)
                            }
                        })
                        .collect();
                    let ep = hits.first();
                    json!({ "address": u, "container": ep.map(|e| &e.container), "project": ep.and_then(|e| e.project.clone()),
                            "state": ep.map(|e| &e.state), "health": ep.and_then(|e| e.health.clone()),
                            "matches": container_host(u).map(|_| hits.len()) })
                })
                .collect();
            let site_certs: Vec<&CertInfo> = certs
                .iter()
                .filter(|c| {
                    s.hosts.iter().any(|h| {
                        h == &c.domain
                            || (c.domain.starts_with("*.") && h.ends_with(&c.domain[1..]) && h.matches('.').count() == c.domain.matches('.').count())
                    })
                })
                .collect();
            let up: Vec<Value> = s
                .hosts
                .iter()
                .filter_map(|h| uptime.iter().find(|(url, ..)| url == &format!("https://{h}")))
                .map(|(url, up, status, latency)| json!({ "url": url, "up": up, "status": status, "latency_ms": latency }))
                .collect();
            json!({ "site": s, "upstreams": upstreams, "certs": site_certs, "uptime": up })
        })
        .collect();
    let caddy = eps.iter().find(|e| e.container == st.cfg.caddy_container);
    let problems = proxy_problems(&st).await.unwrap_or_default();
    Ok(Json(json!({
        "sites": sites,
        "problems": problems,
        "certs": certs,
        "file": st.cfg.caddyfile,
        "mtime": mtime,
        "caddy": { "container": st.cfg.caddy_container, "state": caddy.map(|c| &c.state), "networks": caddy_networks },
        "cloudflare_dns": st.cfg.cloudflare_api_token.is_some() && st.cfg.public_ip.is_some(),
    })))
}

/// Containers Caddy can reach, with their exposed ports, for the wizard.
pub async fn upstreams(State(st): State<AppState>, _u: AuthUser) -> ApiResult<Json<Value>> {
    let eps = endpoints(&st).await;
    let nets: Vec<String> = eps
        .iter()
        .find(|e| e.container == st.cfg.caddy_container)
        .map(|e| e.networks.clone())
        .unwrap_or_default();
    let list: Vec<Value> = eps
        .iter()
        .filter(|e| e.container != st.cfg.caddy_container && e.state == "running")
        .filter(|e| e.networks.iter().any(|n| nets.contains(n)))
        .map(|e| json!({ "container": e.container, "project": e.project, "ports": e.ports, "names": e.names }))
        .collect();
    Ok(Json(json!({ "networks": nets, "containers": list })))
}

pub async fn add(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Json(mut site): Json<NewSite>,
) -> ApiResult<Json<Value>> {
    u.require_sudo()?;
    site.domain = site.domain.trim().to_ascii_lowercase();
    site.upstream = site.upstream.trim().to_string();
    if !caddyfile::valid_domain(&site.domain) {
        return Err(ApiError::BadRequest("некорректный домен".into()));
    }
    if !caddyfile::valid_upstream(&site.upstream) {
        return Err(ApiError::BadRequest(
            "upstream должен быть host:port".into(),
        ));
    }
    let env = caddy_env(&st).await;
    let s = site.clone();
    let Json(mut res) = apply(
        &st,
        &u,
        &client,
        "domain.add",
        &site.domain,
        move |t| caddyfile::add_site(t, &s, &env),
        None,
    )
    .await?;
    if res["ok"] == true {
        let mut extra = Vec::new();
        if let (Some(token), Some(ip)) = (&st.cfg.cloudflare_api_token, &st.cfg.public_ip) {
            if !site.domain.starts_with("*.") {
                extra.push(
                    match crate::cloudflare::ensure_a_record(&st.http, token, &site.domain, ip)
                        .await
                    {
                        Ok(m) => m,
                        Err(e) => format!("DNS: не удалось создать запись: {e:#}"),
                    },
                );
            }
        }
        if !site.domain.starts_with("*.") {
            let _ = sqlx::query("INSERT INTO uptime_checks (url, name, source) VALUES ($1, $2, 'caddy') ON CONFLICT (url) DO NOTHING")
                .bind(format!("https://{}", site.domain))
                .bind(&site.domain)
                .execute(&st.db)
                .await;
        }
        if let Some(steps) = res["steps"].as_array_mut() {
            steps.extend(extra.into_iter().map(Value::from));
        }
    }
    Ok(Json(res))
}

pub async fn remove(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Path(domain): Path<String>,
) -> ApiResult<Json<Value>> {
    u.require_sudo()?;
    let d = domain.clone();
    let res = apply(
        &st,
        &u,
        &client,
        "domain.remove",
        &domain,
        move |t| caddyfile::remove_site(t, &d),
        None,
    )
    .await?;
    if res["ok"] == true {
        let _ = sqlx::query("DELETE FROM uptime_checks WHERE url = $1 AND source = 'caddy'")
            .bind(format!("https://{domain}"))
            .execute(&st.db)
            .await;
    }
    Ok(res)
}

/// Preview of the block the wizard would add.
pub async fn preview(_u: AuthUser, Json(site): Json<NewSite>) -> Json<Value> {
    Json(json!({ "block": caddyfile::render(&site) }))
}

pub async fn get_file(State(st): State<AppState>, _u: AuthUser) -> ApiResult<Json<Value>> {
    let (content, mtime) = read_caddyfile(&st).await?;
    Ok(Json(
        json!({ "path": st.cfg.caddyfile, "content": content, "mtime": mtime }),
    ))
}

#[derive(Deserialize)]
pub struct SaveReq {
    content: String,
    mtime: i64,
}

pub async fn save_file(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Json(req): Json<SaveReq>,
) -> ApiResult<Json<Value>> {
    u.require_sudo()?;
    let mut content = req.content;
    if !content.ends_with('\n') {
        content.push('\n');
    }
    apply(
        &st,
        &u,
        &client,
        "caddyfile.save",
        &st.cfg.caddyfile.clone(),
        move |_| Ok(content),
        Some(req.mtime),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(name: &str, image: &str, nets: &[(&str, &[&str])]) -> Endpoint {
        Endpoint {
            container: name.into(),
            project: None,
            image: image.into(),
            state: "running".into(),
            health: None,
            names: vec![],
            net_names: nets
                .iter()
                .map(|(n, a)| (n.to_string(), a.iter().map(|s| s.to_string()).collect()))
                .collect(),
            ports: vec![],
            networks: nets.iter().map(|(n, _)| n.to_string()).collect(),
        }
    }

    #[test]
    fn finds_network_problems() {
        let text = "a.example.com {\n\treverse_proxy web:80\n}\nb.example.com {\n\treverse_proxy shop-web:3000\n}\nc.example.com {\n\treverse_proxy gone:1\n}\nd.example.com {\n\treverse_proxy 10.0.0.1:80 localhost:9 https://x.example.org\n}\n";
        let eps = vec![
            ep(
                "edge-caddy-1",
                "caddy",
                &[("edge", &["edge-caddy-1", "caddy"])],
            ),
            ep("a-web-1", "nginx", &[("edge", &["a-web-1", "web"])]),
            ep(
                "shop-web-1",
                "x",
                &[("edge", &["shop-web-1", "web", "shop-web"])],
            ),
            // `web` only on its own network: does not count.
            ep("b-web-1", "x", &[("b_default", &["b-web-1", "web"])]),
            ep("b-db-1", "postgres:18-alpine", &[("b_default", &["db"])]),
            ep("c-redis-1", "redis:8", &[("edge", &["redis"])]),
        ];
        let p = network_problems(text, &HashMap::new(), &eps, "edge-caddy-1");
        let kinds: Vec<_> = p.iter().map(|x| (x.kind, x.subject.as_str())).collect();
        assert_eq!(
            kinds,
            [
                ("ambiguous", "web"),
                ("missing", "gone"),
                ("datastore", "c-redis-1")
            ]
        );
        assert!(p[0].message.contains("a-web-1, shop-web-1"));
    }

    #[test]
    fn datastore_images() {
        assert!(is_datastore("postgres:17-alpine@sha256:abc"));
        assert!(is_datastore("docker.io/library/redis:8"));
        assert!(!is_datastore("local/caddy-cloudflare:2"));
        assert!(!is_datastore("ghcr.io/x/shop-backend:sha-1"));
    }

    #[test]
    fn errors() {
        let out = r#"{"level":"info","msg":"using config from file"}
{"level":"error","ts":1,"msg":"adapting config using caddyfile: syntax error: unexpected token 'reverse_proxy', expecting '}', at /dev/stdin:2 import chain: ['']"}"#;
        assert!(caddy_error(out).contains("unexpected token"));
        assert_eq!(caddy_error("Error: plain text"), "Error: plain text");
        assert_eq!(upstream_host("http://bot:8080"), "bot");
        assert_eq!(upstream_host("styk-panel:8080"), "styk-panel");
    }
}
