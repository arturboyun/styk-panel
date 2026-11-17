//! Alert engine. Every minute (and right after interesting Docker events) the built-in
//! rules are evaluated into a set of conditions; new ones open a `firing` event, ones
//! that disappear are `resolved`. Only transitions notify, plus a reminder for
//! unacknowledged critical alerts. One-off events (OOM kill, SSH login, failed job) are
//! stored already resolved.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use bollard::container::ListContainersOptions;
use bollard::system::EventsOptions;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use styk_proto::{Method, SshActivity, Updates};

use crate::notify::{self, escape};
use crate::settings;
use crate::state::AppState;

pub const PREFS_KEY: &str = "alerts.prefs";
const SSH_SEEN_KEY: &str = "alerts.ssh_last_ts";
const SUMMARY_KEY: &str = "alerts.summary_last";

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    State,
    Event,
}

#[derive(Clone, Serialize)]
pub struct RuleDef {
    pub key: &'static str,
    pub group: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub severity: &'static str,
    pub kind: RuleKind,
    pub defaults: Value,
}

pub fn rules() -> Vec<RuleDef> {
    use RuleKind::*;
    let r = |key, group, title, description, severity, kind, defaults| RuleDef {
        key,
        group,
        title,
        description,
        severity,
        kind,
        defaults,
    };
    vec![
        r(
            "cpu_high",
            "Сервер",
            "Высокая загрузка CPU",
            "Средняя загрузка выше порога дольше N минут",
            "warning",
            State,
            json!({ "threshold": 90, "minutes": 10 }),
        ),
        r(
            "mem_high",
            "Сервер",
            "Мало памяти",
            "Занятая RAM выше порога дольше N минут",
            "warning",
            State,
            json!({ "threshold": 90, "minutes": 10 }),
        ),
        r(
            "disk_high",
            "Сервер",
            "Заканчивается диск",
            "Заполнение файловой системы (warning / critical)",
            "warning",
            State,
            json!({ "warning": 85, "critical": 95 }),
        ),
        r(
            "inodes_high",
            "Сервер",
            "Заканчиваются inodes",
            "Доля занятых inodes",
            "warning",
            State,
            json!({ "threshold": 90 }),
        ),
        r(
            "agent_down",
            "Сервер",
            "Нет связи с агентом",
            "Метрики не обновлялись больше 2 минут",
            "critical",
            State,
            json!({}),
        ),
        r(
            "container_down",
            "Контейнеры",
            "Контейнер упал",
            "Контейнер завершился с ошибкой, хотя проект запущен",
            "critical",
            State,
            json!({}),
        ),
        r(
            "container_unhealthy",
            "Контейнеры",
            "Контейнер unhealthy",
            "Healthcheck не проходит дольше N минут",
            "warning",
            State,
            json!({ "minutes": 3 }),
        ),
        r(
            "container_restarting",
            "Контейнеры",
            "Рестарт-луп",
            "Контейнер перезапускается: N падений за 10 минут или состояние restarting",
            "critical",
            State,
            json!({ "count": 3 }),
        ),
        r(
            "container_oom",
            "Контейнеры",
            "OOM kill",
            "Процесс в контейнере убит из-за лимита памяти",
            "critical",
            Event,
            json!({}),
        ),
        r(
            "cert_expiry",
            "Домены",
            "Сертификат скоро истекает",
            "Осталось меньше N дней (меньше 3 — critical)",
            "warning",
            State,
            json!({ "days": 14 }),
        ),
        r(
            "caddy_stale",
            "Домены",
            "Caddy не видит Caddyfile",
            "Файл в контейнере Caddy отличается от файла на хосте: правки не применяются",
            "critical",
            State,
            json!({}),
        ),
        r(
            "proxy_network",
            "Домены",
            "Проблема в общей сети Caddy",
            "Адрес из Caddyfile не найден или совпадает у нескольких контейнеров; база или кэш в общей сети",
            "warning",
            State,
            json!({}),
        ),
        r(
            "uptime_down",
            "Домены",
            "Сайт недоступен",
            "N проверок подряд с ошибкой или статусом ≥ 500",
            "critical",
            State,
            json!({ "failures": 3 }),
        ),
        r(
            "backup_stale",
            "Бэкапы",
            "Бэкап давно не делался",
            "backup-all не завершался успешно дольше N часов",
            "critical",
            State,
            json!({ "hours": 26 }),
        ),
        r(
            "backup_failed",
            "Бэкапы",
            "Бэкап завершился с ошибкой",
            "Последний запуск backup-all.service неуспешен",
            "critical",
            State,
            json!({}),
        ),
        r(
            "restore_test",
            "Бэкапы",
            "Проверка восстановления не прошла",
            "Последний тест восстановления из R2 упал или не запускался дольше N дней",
            "critical",
            State,
            json!({ "days": 8 }),
        ),
        r(
            "job_failed",
            "Бэкапы",
            "Задача завершилась ошибкой",
            "Любая задача панели (дамп, restore, apt, compose)",
            "warning",
            Event,
            json!({}),
        ),
        r(
            "ssh_bruteforce",
            "Безопасность",
            "Всплеск перебора SSH",
            "Неудачных входов за последний час больше порога",
            "warning",
            State,
            json!({ "per_hour": 1500 }),
        ),
        r(
            "ssh_login",
            "Безопасность",
            "Вход по SSH",
            "Каждый успешный вход; ключи из списка игнорируются (например, deploy-ключ Actions)",
            "info",
            Event,
            json!({ "ignore_keys": [] }),
        ),
        r(
            "updates_security",
            "Безопасность",
            "Есть обновления безопасности",
            "apt сообщает о security-обновлениях",
            "warning",
            State,
            json!({}),
        ),
        r(
            "reboot_required",
            "Безопасность",
            "Нужна перезагрузка",
            "/var/run/reboot-required",
            "warning",
            State,
            json!({}),
        ),
        r(
            "docker_reclaimable",
            "Контейнеры",
            "Docker занимает лишнее",
            "Неиспользуемые образы и build cache больше N ГБ",
            "info",
            State,
            json!({ "gb": 20 }),
        ),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prefs {
    pub quiet_enabled: bool,
    pub quiet_from: u32,
    pub quiet_to: u32,
    /// Minutes east of UTC, as sent by the browser.
    pub tz_offset_min: i32,
    pub summary_enabled: bool,
    pub summary_hour: u32,
    pub repeat_hours: u32,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            quiet_enabled: true,
            quiet_from: 23,
            quiet_to: 8,
            tz_offset_min: 180,
            summary_enabled: true,
            summary_hour: 9,
            repeat_hours: 6,
        }
    }
}

impl Prefs {
    fn local_hour(&self, now: DateTime<Utc>) -> u32 {
        let mins = now.timestamp() / 60 + self.tz_offset_min as i64;
        (mins.rem_euclid(1440) / 60) as u32
    }

    pub fn quiet(&self, now: DateTime<Utc>) -> bool {
        if !self.quiet_enabled || self.quiet_from == self.quiet_to {
            return false;
        }
        let h = self.local_hour(now);
        if self.quiet_from < self.quiet_to {
            (self.quiet_from..self.quiet_to).contains(&h)
        } else {
            h >= self.quiet_from || h < self.quiet_to
        }
    }

    fn local_date(&self, now: DateTime<Utc>) -> String {
        (now + chrono::Duration::minutes(self.tz_offset_min as i64))
            .format("%Y-%m-%d")
            .to_string()
    }
}

#[derive(Default)]
pub struct Runtime {
    wake: tokio::sync::Notify,
    deaths: Mutex<HashMap<String, VecDeque<Instant>>>,
    since: Mutex<HashMap<String, Instant>>,
}

impl Runtime {
    /// True once `cond` has held continuously for `mins` minutes.
    fn held(&self, key: &str, cond: bool, mins: u64) -> bool {
        let mut m = self.since.lock().unwrap();
        if !cond {
            m.remove(key);
            return false;
        }
        let t = *m.entry(key.to_string()).or_insert_with(Instant::now);
        t.elapsed() >= Duration::from_secs(mins * 60)
    }

    fn deaths(&self, name: &str) -> usize {
        let mut m = self.deaths.lock().unwrap();
        let q = m.entry(name.to_string()).or_default();
        while q
            .front()
            .is_some_and(|t| t.elapsed() > Duration::from_secs(600))
        {
            q.pop_front();
        }
        q.len()
    }
}

struct Cond {
    rule: &'static str,
    subject: String,
    severity: &'static str,
    message: String,
}

struct Rule {
    def: RuleDef,
    enabled: bool,
    params: Value,
    muted_until: Option<DateTime<Utc>>,
}

impl Rule {
    fn num(&self, k: &str) -> f64 {
        self.params[k]
            .as_f64()
            .or_else(|| self.def.defaults[k].as_f64())
            .unwrap_or(0.0)
    }
}

async fn load_rules(st: &AppState) -> HashMap<&'static str, Rule> {
    let rows: Vec<(String, bool, Value, Option<DateTime<Utc>>)> =
        sqlx::query_as("SELECT key, enabled, params, muted_until FROM alert_rules")
            .fetch_all(&st.db)
            .await
            .unwrap_or_default();
    rules()
        .into_iter()
        .map(|def| {
            let row = rows.iter().find(|r| r.0 == def.key);
            let mut params = def.defaults.clone();
            if let (Some(obj), Some(r)) = (params.as_object_mut(), row) {
                if let Some(o) = r.2.as_object() {
                    for (k, v) in o {
                        obj.insert(k.clone(), v.clone());
                    }
                }
            }
            (
                def.key,
                Rule {
                    enabled: row.is_none_or(|r| r.1),
                    params,
                    muted_until: row.and_then(|r| r.3),
                    def,
                },
            )
        })
        .collect()
}

pub async fn prefs(st: &AppState) -> Prefs {
    settings::get(st, PREFS_KEY).await.unwrap_or_default()
}

pub fn spawn(st: AppState) {
    let s = st.clone();
    tokio::spawn(async move { docker_events(s).await });
    tokio::spawn(async move {
        // Let metrics and uptime gather a first sample.
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            if let Err(e) = evaluate(&st).await {
                tracing::warn!("alerts: {e:#}");
            }
            morning_summary(&st).await;
            heartbeat(&st).await;
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(60)) => {}
                _ = st.alerts.wake.notified() => tokio::time::sleep(Duration::from_secs(5)).await,
            }
        }
    });
}

async fn docker_events(st: AppState) {
    loop {
        let mut filters = HashMap::new();
        filters.insert("type".to_string(), vec!["container".to_string()]);
        filters.insert(
            "event".to_string(),
            vec!["die".into(), "oom".into(), "health_status".into()],
        );
        let mut events = st.docker.events(Some(EventsOptions::<String> {
            filters,
            ..Default::default()
        }));
        while let Some(Ok(ev)) = events.next().await {
            let action = ev.action.unwrap_or_default();
            let attrs = ev.actor.and_then(|a| a.attributes).unwrap_or_default();
            let name = attrs.get("name").cloned().unwrap_or_default();
            match action.as_str() {
                "die" => {
                    st.alerts
                        .deaths
                        .lock()
                        .unwrap()
                        .entry(name)
                        .or_default()
                        .push_back(Instant::now());
                    st.alerts.wake.notify_one();
                }
                "oom" => {
                    let rules = load_rules(&st).await;
                    if rules.get("container_oom").is_some_and(|r| r.enabled) {
                        let msg =
                            format!("Контейнер {name} упёрся в лимит памяти, процесс убит ядром");
                        fire_event(&st, "container_oom", &name, "critical", &msg).await;
                    }
                }
                a if a.starts_with("health_status") => st.alerts.wake.notify_one(),
                _ => {}
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

fn exit_code(status: &str) -> Option<i64> {
    status
        .strip_prefix("Exited (")?
        .split(')')
        .next()?
        .parse()
        .ok()
}

async fn cached<T: serde::de::DeserializeOwned>(
    st: &AppState,
    key: &str,
    ttl: u64,
    m: Method,
) -> Option<T> {
    let agent = st.agent.clone();
    let v = st
        .cache
        .get_or(key, Duration::from_secs(ttl), || async move {
            agent
                .call_timeout::<Value>(m, Duration::from_secs(60))
                .await
        })
        .await
        .ok()?;
    serde_json::from_value(v).ok()
}

/// Returns the conditions plus the set of rules whose data could be gathered.
async fn conditions(
    st: &AppState,
    rules: &HashMap<&'static str, Rule>,
) -> (Vec<Cond>, HashSet<&'static str>) {
    let mut out = Vec::new();
    let mut ok: HashSet<&'static str> = HashSet::new();
    let on = |k: &str| rules.get(k).is_some_and(|r| r.enabled);
    let rt = &st.alerts;
    let now_ms = Utc::now().timestamp_millis();

    // Host metrics.
    let latest = st.metrics.latest().await;
    let fresh = latest.as_ref().is_some_and(|s| now_ms - s.ts < 120_000);
    ok.insert("agent_down");
    if on("agent_down") && !fresh {
        out.push(Cond {
            rule: "agent_down",
            subject: "host".into(),
            severity: "critical",
            message: st
                .metrics
                .error()
                .await
                .unwrap_or_else(|| "метрики не обновляются".into()),
        });
    }
    if let Some(s) = latest.filter(|_| fresh) {
        for (key, f) in [("cpu_high", 0usize), ("mem_high", 1)] {
            let Some(r) = rules.get(key).filter(|r| r.enabled) else {
                continue;
            };
            ok.insert(key);
            let mins = r.num("minutes").max(1.0);
            let samples = st.metrics.recent(now_ms - (mins * 60_000.0) as i64).await;
            let vals: Vec<f64> = samples
                .iter()
                .map(|s| {
                    if f == 0 {
                        s.cpu_pct as f64
                    } else {
                        s.mem_used as f64 * 100.0 / s.mem_total.max(1) as f64
                    }
                })
                .collect();
            let enough = vals.len() as f64 >= mins * 12.0 * 0.8;
            let avg = vals.iter().sum::<f64>() / vals.len().max(1) as f64;
            if enough && avg > r.num("threshold") {
                out.push(Cond {
                    rule: key,
                    subject: "host".into(),
                    severity: r.def.severity,
                    message: format!(
                        "{} в среднем {avg:.0}% за {mins:.0} мин (порог {:.0}%)",
                        if f == 0 { "CPU" } else { "RAM" },
                        r.num("threshold")
                    ),
                });
            }
        }
        if let Some(r) = rules.get("disk_high").filter(|r| r.enabled) {
            ok.insert("disk_high");
            for fs in &s.filesystems {
                let pct = fs.used as f64 * 100.0 / fs.total.max(1) as f64;
                let sev = if pct >= r.num("critical") {
                    "critical"
                } else if pct >= r.num("warning") {
                    "warning"
                } else {
                    continue;
                };
                let free = (fs.total - fs.used.min(fs.total)) as f64 / 1e9;
                out.push(Cond {
                    rule: "disk_high",
                    subject: fs.mount.clone(),
                    severity: sev,
                    message: format!("{} заполнен на {pct:.0}%, свободно {free:.1} ГБ", fs.mount),
                });
            }
        }
        if let Some(r) = rules.get("inodes_high").filter(|r| r.enabled) {
            ok.insert("inodes_high");
            for fs in s.filesystems.iter().filter(|f| f.inodes_total > 0) {
                let pct = fs.inodes_used as f64 * 100.0 / fs.inodes_total as f64;
                if pct >= r.num("threshold") {
                    out.push(Cond {
                        rule: "inodes_high",
                        subject: fs.mount.clone(),
                        severity: "warning",
                        message: format!("{}: занято {pct:.0}% inodes", fs.mount),
                    });
                }
            }
        }
    }

    // Containers.
    if let Ok(list) = st
        .docker
        .list_containers(Some(ListContainersOptions::<String> {
            all: true,
            ..Default::default()
        }))
        .await
    {
        ok.extend([
            "container_down",
            "container_unhealthy",
            "container_restarting",
        ]);
        let mut active: HashSet<String> = HashSet::new();
        for c in &list {
            if c.state.as_deref() == Some("running") {
                if let Some(p) = c
                    .labels
                    .as_ref()
                    .and_then(|l| l.get("com.docker.compose.project"))
                {
                    active.insert(p.clone());
                }
            }
        }
        for c in &list {
            let name = c
                .names
                .as_ref()
                .and_then(|n| n.first())
                .map(|n| n.trim_start_matches('/').to_string())
                .unwrap_or_default();
            let project = c
                .labels
                .as_ref()
                .and_then(|l| l.get("com.docker.compose.project").cloned());
            let state = c.state.as_deref().unwrap_or("");
            let status = c.status.as_deref().unwrap_or("");
            let in_active = project.as_ref().is_some_and(|p| active.contains(p));
            if on("container_down") && in_active && matches!(state, "exited" | "dead") {
                if let Some(code) = exit_code(status).filter(|&c| c != 0) {
                    out.push(Cond {
                        rule: "container_down",
                        subject: name.clone(),
                        severity: "critical",
                        message: format!("{name} завершился с кодом {code} ({status})"),
                    });
                }
            }
            if let Some(r) = rules.get("container_unhealthy").filter(|r| r.enabled) {
                let unhealthy = state == "running" && status.contains("(unhealthy)");
                if rt.held(
                    &format!("unhealthy:{name}"),
                    unhealthy,
                    r.num("minutes") as u64,
                ) {
                    out.push(Cond {
                        rule: "container_unhealthy",
                        subject: name.clone(),
                        severity: "warning",
                        message: format!("{name}: healthcheck не проходит ({status})"),
                    });
                }
            }
            if let Some(r) = rules.get("container_restarting").filter(|r| r.enabled) {
                let deaths = rt.deaths(&name);
                if state == "restarting" || deaths as f64 >= r.num("count") {
                    out.push(Cond {
                        rule: "container_restarting",
                        subject: name.clone(),
                        severity: "critical",
                        message: format!(
                            "{name} перезапускается: {deaths} падений за 10 минут, сейчас {state}"
                        ),
                    });
                }
            }
        }
    }

    // Certificates of sites that are still in the Caddyfile.
    if on("cert_expiry") {
        if let Ok((text, _)) = crate::api::domains::read_caddyfile(st).await {
            let env = crate::api::domains::caddy_env(st).await;
            let hosts: HashSet<String> =
                crate::caddyfile::sites(&crate::caddyfile::parse(&text), &env)
                    .into_iter()
                    .flat_map(|s| s.hosts)
                    .collect();
            let certs = crate::api::domains::certificates(st).await;
            if !certs.is_empty() {
                ok.insert("cert_expiry");
            }
            let days_param = rules["cert_expiry"].num("days");
            // Caddy's local CA issues 12-hour certificates and renews them itself.
            for c in certs
                .iter()
                .filter(|c| hosts.contains(&c.domain) && !c.issuer.contains("Caddy local"))
            {
                let days = (c.not_after - Utc::now().timestamp()) as f64 / 86400.0;
                if days < days_param {
                    let sev = if days < 3.0 { "critical" } else { "warning" };
                    out.push(Cond {
                        rule: "cert_expiry",
                        subject: c.domain.clone(),
                        severity: sev,
                        message: format!(
                            "Сертификат {} истекает через {days:.0} дн. ({})",
                            c.domain, c.issuer
                        ),
                    });
                }
            }
        }
    }

    // Shared proxy: stale Caddyfile mount, ambiguous/missing upstreams, datastores on edge.
    if on("caddy_stale") || on("proxy_network") {
        if let Ok(problems) = crate::api::domains::proxy_problems(st).await {
            ok.insert("caddy_stale");
            ok.insert("proxy_network");
            for p in problems {
                let (rule, severity) = if p.kind == "stale" {
                    ("caddy_stale", "critical")
                } else {
                    ("proxy_network", "warning")
                };
                if on(rule) {
                    out.push(Cond {
                        rule,
                        subject: format!("{}:{}", p.kind, p.subject),
                        severity,
                        message: p.message,
                    });
                }
            }
        }
    }

    // Uptime.
    if let Some(r) = rules.get("uptime_down").filter(|r| r.enabled) {
        let n = r.num("failures").max(1.0) as i64;
        let rows: Result<Vec<(String, i64, i64, Option<String>, Option<i32>)>, _> = sqlx::query_as(
            "SELECT c.url, count(*) FILTER (WHERE NOT r.up), count(*), max(r.error), max(r.status)
               FROM uptime_checks c
               JOIN LATERAL (SELECT up, error, status FROM uptime_results WHERE check_id = c.id ORDER BY ts DESC LIMIT $1) r ON true
              WHERE c.enabled GROUP BY c.url",
        )
        .bind(n)
        .fetch_all(&st.db)
        .await;
        if let Ok(rows) = rows {
            ok.insert("uptime_down");
            for (url, down, total, err, status) in rows {
                if total >= n && down == total {
                    let why = err
                        .or(status.map(|s| format!("HTTP {s}")))
                        .unwrap_or_default();
                    out.push(Cond {
                        rule: "uptime_down",
                        subject: url.clone(),
                        severity: "critical",
                        message: format!("{url} не отвечает {n} проверок подряд: {why}"),
                    });
                }
            }
        }
    }

    // Backups.
    if on("backup_stale") || on("backup_failed") {
        let info: Option<HashMap<String, String>> = cached(
            st,
            "alerts:backup",
            120,
            Method::UnitInfo {
                unit: "backup-all.service".into(),
            },
        )
        .await;
        if let Some(info) =
            info.filter(|i| i.get("LoadState").map(String::as_str) == Some("loaded"))
        {
            ok.extend(["backup_stale", "backup_failed"]);
            let exited: i64 = info
                .get("ExecMainExitTimestamp")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let running = info.get("ActiveState").map(String::as_str) == Some("activating");
            let result = info.get("Result").cloned().unwrap_or_default();
            if on("backup_failed") && !running && result != "success" && !result.is_empty() {
                out.push(Cond {
                    rule: "backup_failed",
                    subject: "backup-all".into(),
                    severity: "critical",
                    message: format!("backup-all.service: Result={result}"),
                });
            }
            let hours = rules["backup_stale"].num("hours");
            let age_h = (Utc::now().timestamp() - exited) as f64 / 3600.0;
            if on("backup_stale") && !running && (exited == 0 || age_h > hours) {
                out.push(Cond {
                    rule: "backup_stale",
                    subject: "backup-all".into(),
                    severity: "critical",
                    message: format!(
                        "Последний бэкап завершился {age_h:.0} ч назад (порог {hours:.0} ч)"
                    ),
                });
            }
        }
    }

    if on("restore_test") {
        let status: Option<Value> =
            cached(st, "alerts:backup-status", 300, Method::BackupStatus).await;
        if let Some(status) = status {
            ok.insert("restore_test");
            let last = status.get("restore_test").filter(|v| !v.is_null());
            let ts = last.and_then(|l| l["ts"].as_i64()).unwrap_or(0);
            let days = rules["restore_test"].num("days");
            let age_d = (Utc::now().timestamp() - ts) as f64 / 86400.0;
            if let Some(l) = last.filter(|l| l["ok"].as_bool() == Some(false)) {
                out.push(Cond {
                    rule: "restore_test",
                    subject: "restore-test".into(),
                    severity: "critical",
                    message: format!(
                        "Восстановление из R2 не прошло: {}",
                        l["error"]
                            .as_str()
                            .unwrap_or("см. журнал backup-restore-test")
                    ),
                });
            } else if age_d > days {
                out.push(Cond {
                    rule: "restore_test",
                    subject: "restore-test".into(),
                    severity: "critical",
                    message: if ts == 0 {
                        "Восстановление из R2 ещё ни разу не проверялось".into()
                    } else {
                        format!(
                            "Восстановление из R2 не проверялось {age_d:.0} дн. (порог {days:.0})"
                        )
                    },
                });
            }
        }
    }

    // SSH.
    if on("ssh_bruteforce") || on("ssh_login") {
        let act: Option<SshActivity> =
            cached(st, "alerts:ssh", 240, Method::SshActivity { hours: 1 }).await;
        if let Some(a) = act {
            ok.insert("ssh_bruteforce");
            let limit = rules["ssh_bruteforce"].num("per_hour");
            if on("ssh_bruteforce") && a.failed as f64 > limit {
                let top = a
                    .top_ips
                    .first()
                    .map(|i| format!(", больше всех {} ({})", i.ip, i.count))
                    .unwrap_or_default();
                out.push(Cond {
                    rule: "ssh_bruteforce",
                    subject: "host".into(),
                    severity: "warning",
                    message: format!(
                        "{} неудачных входов по SSH за час с {} IP{top}",
                        a.failed, a.unique_ips
                    ),
                });
            }
            if on("ssh_login") {
                ssh_logins(st, &rules["ssh_login"], &a).await;
            }
        }
    }

    // Updates.
    if on("updates_security") || on("reboot_required") {
        let up: Option<Updates> = cached(st, "alerts:updates", 6 * 3600, Method::Updates).await;
        if let Some(u) = up {
            ok.extend(["updates_security", "reboot_required"]);
            let sec: Vec<&str> = u
                .packages
                .iter()
                .filter(|p| p.security)
                .map(|p| p.name.as_str())
                .collect();
            if on("updates_security") && !sec.is_empty() {
                let list = sec.iter().take(8).cloned().collect::<Vec<_>>().join(", ");
                out.push(Cond {
                    rule: "updates_security",
                    subject: "apt".into(),
                    severity: "warning",
                    message: format!("{} security-обновлений: {list}", sec.len()),
                });
            }
            if on("reboot_required") && u.reboot_required {
                out.push(Cond {
                    rule: "reboot_required",
                    subject: "host".into(),
                    severity: "warning",
                    message: format!(
                        "Требуется перезагрузка после обновления: {}",
                        u.reboot_packages.join(", ")
                    ),
                });
            }
        }
    }

    if let Some(r) = rules.get("docker_reclaimable").filter(|r| r.enabled) {
        let v = st
            .cache
            .get_or("alerts:df", Duration::from_secs(3600), || async {
                crate::docker::disk_usage(st)
                    .await
                    .map(|d| serde_json::to_value(d).unwrap_or_default())
            })
            .await;
        if let Ok(d) = v {
            ok.insert("docker_reclaimable");
            let bytes = d["images_reclaimable"].as_f64().unwrap_or(0.0)
                + d["build_cache_reclaimable"].as_f64().unwrap_or(0.0);
            if bytes / 1e9 > r.num("gb") {
                out.push(Cond {
                    rule: "docker_reclaimable",
                    subject: "docker".into(),
                    severity: "info",
                    message: format!(
                        "Можно освободить {:.1} ГБ: Проекты → Очистка Docker",
                        bytes / 1e9
                    ),
                });
            }
        }
    }
    (out, ok)
}

async fn ssh_logins(st: &AppState, rule: &Rule, a: &SshActivity) {
    let last: Option<i64> = settings::get(st, SSH_SEEN_KEY).await;
    let newest = a.accepted.iter().map(|l| l.ts).max();
    let Some(last) = last else {
        // First run: start from now instead of replaying the last hour.
        let _ = settings::set(
            st,
            SSH_SEEN_KEY,
            &newest.unwrap_or_else(|| Utc::now().timestamp()),
        )
        .await;
        return;
    };
    let ignore: Vec<String> =
        serde_json::from_value(rule.params["ignore_keys"].clone()).unwrap_or_default();
    let mut fresh: Vec<_> = a.accepted.iter().filter(|l| l.ts > last).collect();
    fresh.sort_by_key(|l| l.ts);
    for l in &fresh {
        if ignore
            .iter()
            .any(|k| !k.is_empty() && l.key.contains(k.as_str()))
        {
            continue;
        }
        let msg = format!(
            "{} вошёл по SSH с {} ({}{})",
            l.user,
            l.ip,
            l.method,
            if l.key.is_empty() {
                String::new()
            } else {
                format!(", {}", l.key)
            }
        );
        fire_event(
            st,
            "ssh_login",
            &format!("{}@{}", l.user, l.ip),
            "info",
            &msg,
        )
        .await;
    }
    if let Some(n) = newest.filter(|&n| n > last) {
        let _ = settings::set(st, SSH_SEEN_KEY, &n).await;
    }
}

#[derive(sqlx::FromRow)]
struct Open {
    id: i64,
    rule: String,
    subject: String,
    severity: String,
    acked_at: Option<DateTime<Utc>>,
    muted_until: Option<DateTime<Utc>>,
    last_notified_at: Option<DateTime<Utc>>,
}

pub async fn evaluate(st: &AppState) -> anyhow::Result<()> {
    let rules = load_rules(st).await;
    let (conds, evaluated) = conditions(st, &rules).await;
    let open: Vec<Open> = sqlx::query_as(
        "SELECT id, rule, subject, severity, acked_at, muted_until, last_notified_at FROM alert_events WHERE status = 'firing'",
    )
    .fetch_all(&st.db)
    .await?;
    let prefs = prefs(st).await;
    let now = Utc::now();

    for c in &conds {
        match open
            .iter()
            .find(|o| o.rule == c.rule && o.subject == c.subject)
        {
            None => {
                let id: Option<i64> = sqlx::query_scalar(
                    "INSERT INTO alert_events (rule, subject, severity, status, message) VALUES ($1,$2,$3,'firing',$4)
                     ON CONFLICT (rule, subject) WHERE status = 'firing' DO NOTHING RETURNING id",
                )
                .bind(c.rule)
                .bind(&c.subject)
                .bind(c.severity)
                .bind(&c.message)
                .fetch_optional(&st.db)
                .await?;
                if let Some(id) = id {
                    deliver(
                        st, &rules, &prefs, id, c.rule, c.severity, &c.message, false,
                    )
                    .await;
                }
            }
            Some(o) => {
                let escalated = o.severity != "critical" && c.severity == "critical";
                sqlx::query("UPDATE alert_events SET message = $2, severity = $3 WHERE id = $1")
                    .bind(o.id)
                    .bind(&c.message)
                    .bind(c.severity)
                    .execute(&st.db)
                    .await?;
                let remind = c.severity == "critical"
                    && o.acked_at.is_none()
                    && !o.muted_until.is_some_and(|m| m > now)
                    && o.last_notified_at.is_some_and(|t| {
                        now - t > chrono::Duration::hours(prefs.repeat_hours.max(1) as i64)
                    });
                if escalated || remind {
                    deliver(
                        st, &rules, &prefs, o.id, c.rule, c.severity, &c.message, false,
                    )
                    .await;
                }
            }
        }
    }

    for o in &open {
        let still = conds
            .iter()
            .any(|c| c.rule == o.rule && c.subject == o.subject);
        let rule_known = rules.get(o.rule.as_str());
        let can_judge =
            evaluated.contains(o.rule.as_str()) || rule_known.is_none_or(|r| !r.enabled);
        if !still && can_judge {
            sqlx::query(
                "UPDATE alert_events SET status = 'resolved', resolved_at = now() WHERE id = $1",
            )
            .bind(o.id)
            .execute(&st.db)
            .await?;
            if o.last_notified_at.is_some() {
                let msg = format!(
                    "{}: {}",
                    rule_known.map(|r| r.def.title).unwrap_or(&o.rule),
                    o.subject
                );
                deliver(st, &rules, &prefs, o.id, &o.rule, &o.severity, &msg, true).await;
            }
        }
    }
    Ok(())
}

/// One-off event: stored resolved, notified once.
pub async fn fire_event(st: &AppState, rule: &str, subject: &str, severity: &str, message: &str) {
    let rules = load_rules(st).await;
    if rules.get(rule).is_some_and(|r| !r.enabled) {
        return;
    }
    let id: Result<i64, _> = sqlx::query_scalar(
        "INSERT INTO alert_events (rule, subject, severity, status, message, resolved_at) VALUES ($1,$2,$3,'resolved',$4, now()) RETURNING id",
    )
    .bind(rule)
    .bind(subject)
    .bind(severity)
    .bind(message)
    .fetch_one(&st.db)
    .await;
    if let Ok(id) = id {
        let prefs = prefs(st).await;
        deliver(st, &rules, &prefs, id, rule, severity, message, false).await;
    }
}

fn icon(severity: &str, resolved: bool) -> &'static str {
    match (resolved, severity) {
        (true, _) => "✅",
        (_, "critical") => "🔴",
        (_, "warning") => "🟠",
        _ => "🔵",
    }
}

#[allow(clippy::too_many_arguments)]
async fn deliver(
    st: &AppState,
    rules: &HashMap<&'static str, Rule>,
    prefs: &Prefs,
    id: i64,
    rule: &str,
    severity: &str,
    message: &str,
    resolved: bool,
) {
    let now = Utc::now();
    let def = rules.get(rule);
    if def.is_some_and(|r| r.muted_until.is_some_and(|m| m > now)) {
        return;
    }
    if severity != "critical" && prefs.quiet(now) {
        return;
    }
    let title = def.map(|r| r.def.title).unwrap_or(rule);
    let html = if resolved {
        format!(
            "{} <b>Решено</b> · {}",
            icon(severity, true),
            escape(message)
        )
    } else {
        format!(
            "{} <b>{}</b>\n{}",
            icon(severity, false),
            escape(title),
            escape(message)
        )
    };
    // Telegram rejects a button with an empty URL, so without STYK_PUBLIC_URL there is none.
    let open: Vec<Value> = if st.cfg.public_url.is_empty() {
        vec![]
    } else {
        let url = format!("{}/alerts/", st.cfg.public_url.trim_end_matches('/'));
        vec![json!([{ "text": "Открыть панель", "url": url }])]
    };
    let buttons = if resolved || def.is_some_and(|r| r.def.kind == RuleKind::Event) {
        json!(open)
    } else {
        let mut rows = vec![json!([
            { "text": "✓ Принято", "callback_data": format!("ack:{id}") },
            { "text": "🔕 1 ч", "callback_data": format!("mute:{id}:1") },
            { "text": "🔕 24 ч", "callback_data": format!("mute:{id}:24") }
        ])];
        rows.extend(open);
        json!(rows)
    };
    match notify::send(st, &html, buttons).await {
        Ok(true) => {
            let _ = sqlx::query("UPDATE alert_events SET last_notified_at = now() WHERE id = $1")
                .bind(id)
                .execute(&st.db)
                .await;
        }
        Ok(false) => {}
        Err(e) => tracing::warn!("alert notification: {e:#}"),
    }
}

/// Inline-button handler; `user` is the linked panel user.
pub async fn telegram_action(st: &AppState, user: &str, data: &str) -> String {
    let parts: Vec<&str> = data.split(':').collect();
    let id: i64 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let res = match parts.first() {
        Some(&"ack") => {
            sqlx::query("UPDATE alert_events SET acked_by = $2, acked_at = now() WHERE id = $1")
                .bind(id)
                .bind(user)
                .execute(&st.db)
                .await
                .map(|_| "Принято".to_string())
        }
        Some(&"mute") => {
            let h: i64 = parts
                .get(2)
                .and_then(|s| s.parse().ok())
                .unwrap_or(1)
                .clamp(1, 24 * 7);
            sqlx::query("UPDATE alert_events SET muted_until = now() + make_interval(hours => $2::int), acked_by = $3, acked_at = now() WHERE id = $1")
                .bind(id)
                .bind(h as i32)
                .bind(user)
                .execute(&st.db)
                .await
                .map(|_| format!("Без напоминаний {h} ч"))
        }
        _ => return "Неизвестное действие".into(),
    };
    crate::audit::record(
        st,
        crate::audit::Entry {
            user_id: None,
            username: Some(user),
            action: "alert.telegram",
            target: Some(data),
            ip: "telegram",
            ok: res.is_ok(),
            details: json!({}),
        },
    )
    .await;
    res.unwrap_or_else(|e| format!("Ошибка: {e}"))
}

pub async fn summary(st: &AppState) -> String {
    let firing: Vec<(String, String)> = sqlx::query_as(
        "SELECT severity, message FROM alert_events WHERE status = 'firing' ORDER BY severity, started_at",
    )
    .fetch_all(&st.db)
    .await
    .unwrap_or_default();
    let failed_jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE status IN ('failed','lost') AND started_at > now() - interval '24 hours'")
        .fetch_one(&st.db)
        .await
        .unwrap_or(0);
    let down: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM uptime_checks c WHERE c.enabled AND (SELECT NOT up FROM uptime_results WHERE check_id = c.id ORDER BY ts DESC LIMIT 1)",
    )
    .fetch_one(&st.db)
    .await
    .unwrap_or(0);
    let mut s = String::from("☀️ <b>Сводка Styk Panel</b>\n");
    if let Some(m) = st.metrics.latest().await {
        let mem = m.mem_used as f64 * 100.0 / m.mem_total.max(1) as f64;
        let root = m.filesystems.iter().find(|f| f.mount == "/");
        s.push_str(&format!("\nCPU {:.0}% · RAM {mem:.0}%", m.cpu_pct));
        if let Some(r) = root {
            s.push_str(&format!(
                " · диск / {:.0}%",
                r.used as f64 * 100.0 / r.total.max(1) as f64
            ));
        }
        let running = m.containers.len();
        s.push_str(&format!("\nКонтейнеров запущено: {running}"));
    }
    if let Some(a) =
        cached::<SshActivity>(st, "summary:ssh", 600, Method::SshActivity { hours: 24 }).await
    {
        s.push_str(&format!(
            "\nSSH: {} неудачных входов за сутки, {} успешных",
            a.failed,
            a.accepted.len()
        ));
    }
    if down > 0 {
        s.push_str(&format!("\n🔴 Недоступно сайтов: {down}"));
    }
    if failed_jobs > 0 {
        s.push_str(&format!("\n🟠 Задач с ошибкой за сутки: {failed_jobs}"));
    }
    if firing.is_empty() {
        s.push_str("\n\n✅ Активных алертов нет");
    } else {
        s.push_str(&format!("\n\n<b>Активные алерты ({})</b>", firing.len()));
        for (sev, msg) in firing.iter().take(15) {
            s.push_str(&format!("\n{} {}", icon(sev, false), escape(msg)));
        }
    }
    s
}

async fn morning_summary(st: &AppState) {
    let p = prefs(st).await;
    let now = Utc::now();
    if !p.summary_enabled || p.local_hour(now) != p.summary_hour {
        return;
    }
    let today = p.local_date(now);
    let last: Option<String> = settings::get(st, SUMMARY_KEY).await;
    if last.as_deref() == Some(today.as_str()) {
        return;
    }
    let _ = settings::set(st, SUMMARY_KEY, &today).await;
    let html = summary(st).await;
    if let Err(e) = notify::send(st, &html, json!([])).await {
        tracing::warn!("morning summary: {e:#}");
    }
}

async fn heartbeat(st: &AppState) {
    let Some(url) = &st.cfg.heartbeat_url else {
        return;
    };
    let fresh = st
        .metrics
        .latest()
        .await
        .is_some_and(|s| Utc::now().timestamp_millis() - s.ts < 120_000);
    if fresh {
        if let Err(e) = st.http.get(url).send().await {
            tracing::warn!("heartbeat: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_hours() {
        let p = Prefs {
            tz_offset_min: 180,
            quiet_from: 23,
            quiet_to: 8,
            ..Default::default()
        };
        let at = |h: u32| {
            DateTime::parse_from_rfc3339(&format!("2026-11-03T{h:02}:30:00Z"))
                .unwrap()
                .with_timezone(&Utc)
        };
        assert!(p.quiet(at(21))); // 00:30 local
        assert!(p.quiet(at(4))); // 07:30
        assert!(!p.quiet(at(5))); // 08:30
        assert!(!p.quiet(at(12)));
        assert!(p.quiet(at(20))); // 23:30
        let day = Prefs {
            quiet_from: 9,
            quiet_to: 18,
            tz_offset_min: 0,
            ..Default::default()
        };
        assert!(day.quiet(at(10)));
        assert!(!day.quiet(at(19)));
    }

    #[test]
    fn exit_codes() {
        assert_eq!(exit_code("Exited (137) 5 minutes ago"), Some(137));
        assert_eq!(exit_code("Exited (0) 2 days ago"), Some(0));
        assert_eq!(exit_code("Up 3 hours"), None);
    }

    #[test]
    fn rule_keys_unique() {
        let r = rules();
        let keys: HashSet<_> = r.iter().map(|r| r.key).collect();
        assert_eq!(keys.len(), r.len());
    }

    #[test]
    fn held_resets() {
        let rt = Runtime::default();
        assert!(rt.held("x", true, 0));
        assert!(!rt.held("x", false, 0));
        assert!(!rt.held("y", true, 5));
    }
}
