//! Security page: SSH activity, sshd settings with advice, listening sockets, firewall,
//! pending updates. Read-only except for "install updates".

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use styk_proto::{
    AccessConfig, AccessState, ListeningSocket, Method, SecurityTools, Task, Updates,
};

use crate::audit;
use crate::auth::{AuthUser, ClientInfo};
use crate::error::{agent_error, ApiError, ApiResult};
use crate::jobs;
use crate::state::AppState;

#[derive(Serialize)]
pub struct Finding {
    /// critical | warning | ok
    level: &'static str,
    title: String,
    detail: String,
    fix: Option<String>,
}

fn f(level: &'static str, title: &str, detail: &str, fix: Option<&str>) -> Finding {
    Finding {
        level,
        title: title.into(),
        detail: detail.into(),
        fix: fix.map(Into::into),
    }
}

const SSHD_KEYS: &[&str] = &[
    "port",
    "permitrootlogin",
    "passwordauthentication",
    "kbdinteractiveauthentication",
    "pubkeyauthentication",
    "maxauthtries",
    "x11forwarding",
    "allowtcpforwarding",
    "permitemptypasswords",
    "usepam",
    "logingracetime",
    "maxstartups",
];

/// Ports that are expected to be reachable from the internet.
const EXPECTED_PUBLIC: &[u16] = &[22, 80, 443];

fn findings(
    sshd: &BTreeMap<String, String>,
    tools: &SecurityTools,
    ports: &[ListeningSocket],
    updates: Option<&Updates>,
) -> Vec<Finding> {
    let mut out = Vec::new();
    let get = |k: &str| sshd.get(k).map(String::as_str).unwrap_or("");
    let ssh_fix = "Создайте /etc/ssh/sshd_config.d/10-hardening.conf с «PasswordAuthentication no» и «KbdInteractiveAuthentication no», проверьте `sshd -t` и выполните `systemctl reload ssh` (не закрывая текущую сессию).";
    if get("passwordauthentication") == "yes" {
        out.push(f(
            "critical",
            "Вход по паролю по SSH включён",
            "Боты непрерывно перебирают пароли. Входите только по ключам: ключи уже используются.",
            Some(ssh_fix),
        ));
    } else {
        out.push(f(
            "ok",
            "Вход по паролю отключён",
            "SSH принимает только ключи.",
            None,
        ));
    }
    match get("permitrootlogin") {
        "yes" => out.push(f(
            "critical",
            "root может входить по паролю",
            "PermitRootLogin yes разрешает перебор пароля root.",
            Some("PermitRootLogin prohibit-password (или no, если есть отдельный пользователь с sudo)."),
        )),
        "prohibit-password" | "without-password" => out.push(f(
            "ok",
            "root входит только по ключу",
            "PermitRootLogin prohibit-password.",
            None,
        )),
        _ => {}
    }
    if get("permitemptypasswords") == "yes" {
        out.push(f(
            "critical",
            "Разрешены пустые пароли",
            "PermitEmptyPasswords yes.",
            Some("PermitEmptyPasswords no"),
        ));
    }
    if get("maxauthtries").parse::<u32>().unwrap_or(6) > 3 {
        out.push(f(
            "warning",
            "Много попыток входа за соединение",
            &format!(
                "MaxAuthTries {}: ботам выгодно держать соединение.",
                get("maxauthtries")
            ),
            Some("MaxAuthTries 3"),
        ));
    }
    if !tools.fail2ban && !tools.crowdsec {
        out.push(f(
            "warning",
            "Нет автоматической блокировки перебора",
            "Ни fail2ban, ни CrowdSec не запущены: IP с тысячами неудачных попыток не блокируются.",
            Some("apt install fail2ban (джейл sshd включён по умолчанию) или CrowdSec с bouncer-ом для nftables."),
        ));
    } else {
        out.push(f(
            "ok",
            "Перебор блокируется",
            if tools.fail2ban {
                "fail2ban активен."
            } else {
                "CrowdSec активен."
            },
            None,
        ));
    }
    // One finding per process and run of consecutive ports: IPv4/IPv6 listeners and
    // published ranges (e.g. 20000-20099) would otherwise produce a card per socket.
    let mut groups: BTreeMap<(&str, &str), (BTreeSet<u16>, BTreeSet<&str>)> = BTreeMap::new();
    for p in ports
        .iter()
        .filter(|p| p.public && !EXPECTED_PUBLIC.contains(&p.port))
    {
        if p.proto == "udp" && matches!(p.port, 68 | 123 | 323 | 546) {
            continue;
        }
        let g = groups
            .entry((p.proto.as_str(), p.process.as_str()))
            .or_default();
        g.0.insert(p.port);
        g.1.insert(p.address.as_str());
    }
    for ((proto, process), (ports, addrs)) in groups {
        let docker = process == "docker-proxy";
        let addrs = addrs.into_iter().collect::<Vec<_>>().join(", ");
        for (lo, hi) in port_runs(&ports) {
            let title = if lo == hi {
                format!("Порт {lo}/{proto} открыт наружу")
            } else {
                format!(
                    "Порты {lo}–{hi}/{proto} открыты наружу ({} шт.)",
                    hi - lo + 1
                )
            };
            out.push(f(
                "warning",
                &title,
                &format!(
                    "{} слушает {addrs}{}",
                    if process.is_empty() { "?" } else { process },
                    if docker { ": опубликован Docker, правила ufw его не закрывают." } else { "." }
                ),
                Some(if docker {
                    "Публикуйте порт только на 127.0.0.1 (ports: \"127.0.0.1:PORT:PORT\") или уберите ports и ходите через сеть Caddy."
                } else {
                    "Привяжите сервис к 127.0.0.1 или закройте порт в nftables."
                }),
            ));
        }
    }
    if let Some(u) = updates {
        let sec = u.packages.iter().filter(|p| p.security).count();
        if sec > 0 {
            out.push(f(
                "warning",
                &format!("{sec} обновлений безопасности"),
                "Установите их на вкладке «Обновления».",
                None,
            ));
        }
        if u.reboot_required {
            out.push(f(
                "warning",
                "Нужна перезагрузка",
                &format!("После обновления: {}.", u.reboot_packages.join(", ")),
                Some("Перезагрузите сервер в спокойное время: `systemctl reboot` (все контейнеры поднимутся сами, если у них restart: unless-stopped)."),
            ));
        }
    }
    out.sort_by_key(|f| match f.level {
        "critical" => 0,
        "warning" => 1,
        _ => 2,
    });
    out
}

/// Consecutive runs in a sorted port set: {80, 81, 82, 90} → [(80, 82), (90, 90)].
fn port_runs(ports: &BTreeSet<u16>) -> Vec<(u16, u16)> {
    let mut runs: Vec<(u16, u16)> = Vec::new();
    for &p in ports {
        match runs.last_mut() {
            Some((_, hi)) if hi.checked_add(1) == Some(p) => *hi = p,
            _ => runs.push((p, p)),
        }
    }
    runs
}

async fn cached(st: &AppState, key: &str, ttl: u64, m: Method) -> ApiResult<Value> {
    let agent = st.agent.clone();
    st.cache
        .get_or(key, Duration::from_secs(ttl), || async move {
            agent
                .call_timeout::<Value>(m, Duration::from_secs(60))
                .await
                .map_err(agent_error)
        })
        .await
}

pub async fn overview(State(st): State<AppState>, _u: AuthUser) -> ApiResult<Json<Value>> {
    let (sshd, tools, ports, updates) = tokio::join!(
        cached(&st, "sec:sshd", 60, Method::SshdConfig),
        cached(&st, "sec:tools", 60, Method::SecurityTools),
        cached(&st, "sec:ports", 20, Method::ListeningPorts),
        cached(&st, "sec:updates", 300, Method::Updates),
    );
    // Each section degrades on its own: a failing tool must not hide the others.
    let mut errors = serde_json::Map::new();
    let mut take = |name: &str, r: ApiResult<Value>| match r {
        Ok(v) => Some(v),
        Err(e) => {
            errors.insert(name.into(), e.to_string().into());
            None
        }
    };
    let sshd_ok = take("sshd", sshd);
    let sshd: BTreeMap<String, String> = sshd_ok
        .clone()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let tools: SecurityTools = take("tools", tools)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let ports: Vec<ListeningSocket> = take("ports", ports)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let updates: Option<Updates> =
        take("updates", updates).and_then(|u| serde_json::from_value(u).ok());
    let mut findings = findings(&sshd, &tools, &ports, updates.as_ref());
    if sshd_ok.is_none() {
        findings.retain(|f| {
            !f.title.contains("парол") && !f.title.contains("root") && !f.title.contains("попыток")
        });
    }
    let sshd: BTreeMap<&str, &str> = SSHD_KEYS
        .iter()
        .filter_map(|k| Some((*k, sshd.get(*k)?.as_str())))
        .collect();
    Ok(Json(json!({
        "findings": findings,
        "sshd": sshd,
        "tools": tools,
        "ports": ports,
        "updates": updates,
        "errors": errors,
    })))
}

#[derive(Deserialize)]
pub struct SshQuery {
    hours: Option<u32>,
}

pub async fn ssh(
    State(st): State<AppState>,
    _u: AuthUser,
    Query(q): Query<SshQuery>,
) -> ApiResult<Json<Value>> {
    let hours = q.hours.unwrap_or(24).clamp(1, 24 * 30);
    let key = format!("sec:ssh:{hours}");
    Ok(Json(
        cached(&st, &key, 60, Method::SshActivity { hours }).await?,
    ))
}

pub async fn firewall(State(st): State<AppState>, _u: AuthUser) -> ApiResult<Json<Value>> {
    Ok(Json(cached(&st, "sec:nft", 30, Method::Firewall).await?))
}

pub async fn updates(State(st): State<AppState>, _u: AuthUser) -> ApiResult<Json<Value>> {
    Ok(Json(
        cached(&st, "sec:updates", 300, Method::Updates).await?,
    ))
}

pub async fn apply_updates(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
) -> ApiResult<Json<Value>> {
    u.require_sudo()?;
    let id = jobs::start(
        &st,
        jobs::Actor::user(&u, &client),
        "apt-upgrade",
        "host",
        Task::AptUpgrade,
    )
    .await?;
    st.cache.invalidate("sec:updates");
    Ok(Json(json!({ "job": id })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advice() {
        let sshd: BTreeMap<String, String> = [
            ("passwordauthentication", "yes"),
            ("permitrootlogin", "prohibit-password"),
            ("maxauthtries", "6"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let ports = vec![
            ListeningSocket {
                proto: "tcp".into(),
                address: "0.0.0.0".into(),
                port: 22,
                process: "sshd".into(),
                public: true,
            },
            ListeningSocket {
                proto: "tcp".into(),
                address: "0.0.0.0".into(),
                port: 5432,
                process: "docker-proxy".into(),
                public: true,
            },
            ListeningSocket {
                proto: "tcp".into(),
                address: "127.0.0.1".into(),
                port: 6379,
                process: "redis".into(),
                public: false,
            },
        ];
        let fs = findings(&sshd, &SecurityTools::default(), &ports, None);
        assert_eq!(fs[0].level, "critical");
        assert!(fs[0].title.contains("паролю"));
        assert!(fs.iter().any(|f| f.title.contains("5432")));
        assert!(!fs
            .iter()
            .any(|f| f.title.contains("6379") || f.title.contains("Порт 22")));
        assert!(fs.iter().any(|f| f.title.contains("блокировки")));
    }

    #[test]
    fn port_ranges_collapse() {
        let sock = |address: &str, port| ListeningSocket {
            proto: "tcp".into(),
            address: address.into(),
            port,
            process: "docker-proxy".into(),
            public: true,
        };
        let mut ports = vec![sock("0.0.0.0", 7835), sock("::", 7835)];
        for p in 20000..20100 {
            ports.push(sock("0.0.0.0", p));
            ports.push(sock("::", p));
        }
        let fs = findings(&BTreeMap::new(), &SecurityTools::default(), &ports, None);
        let titles: Vec<_> = fs
            .iter()
            .filter(|f| f.title.contains("наружу"))
            .map(|f| f.title.as_str())
            .collect();
        assert_eq!(
            titles,
            [
                "Порт 7835/tcp открыт наружу",
                "Порты 20000–20099/tcp открыты наружу (100 шт.)"
            ]
        );
        assert_eq!(
            port_runs(&[1, 2, 3, 7, 9, 10].into_iter().collect()),
            [(1, 3), (7, 7), (9, 10)]
        );
    }
}

// ---- IP access rules -----------------------------------------------------------

pub async fn access(State(st): State<AppState>, _u: AuthUser) -> ApiResult<Json<AccessState>> {
    Ok(Json(
        st.agent
            .call(Method::AccessGet)
            .await
            .map_err(agent_error)?,
    ))
}

#[derive(Deserialize)]
pub struct AccessReq {
    config: AccessConfig,
    #[serde(default)]
    force: bool,
}

pub async fn save_access(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Json(req): Json<AccessReq>,
) -> ApiResult<Json<AccessState>> {
    u.require_sudo()?;
    // The agent cannot know who is using the panel; this guard can.
    if req.config.block.iter().any(|e| e.cidr.trim() == client.ip) {
        return Err(ApiError::BadRequest(format!(
            "{} — это ваш текущий адрес, панель станет недоступна",
            client.ip
        )));
    }
    let res = st
        .agent
        .call::<AccessState>(Method::AccessSet {
            config: req.config.clone(),
            force: req.force,
        })
        .await
        .map_err(agent_error);
    audit::user_action(
        &st,
        &u,
        &client,
        "access.save",
        "host",
        res.is_ok(),
        json!({
            "ssh_allow_enabled": req.config.ssh_allow_enabled,
            "ssh_allow": req.config.ssh_allow.iter().map(|e| &e.cidr).collect::<Vec<_>>(),
            "block": req.config.block.iter().map(|e| &e.cidr).collect::<Vec<_>>(),
            "force": req.force,
            "error": res.as_ref().err().map(|e| e.to_string()),
        }),
    )
    .await;
    st.cache.invalidate("sec:nft");
    Ok(Json(res?))
}
