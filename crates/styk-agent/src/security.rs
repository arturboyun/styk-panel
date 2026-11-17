//! Read-only views of the host's exposure: SSH activity, listening sockets, firewall,
//! sshd settings, pending updates.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::Result;
use styk_proto::{
    FirewallTable, ListeningSocket, SecurityTools, SshActivity, SshIp, SshLogin, Updates,
    UpgradablePackage,
};

use crate::util::{run, try_run};

pub fn ssh_activity(hours: u32) -> Result<SshActivity> {
    let hours = hours.clamp(1, 24 * 31);
    let since = format!("-{hours}h");
    let out = run(
        "journalctl",
        &[
            "-u",
            "ssh",
            "-u",
            "sshd",
            "--since",
            &since,
            "-o",
            "short-unix",
            "--no-pager",
            "-q",
        ],
    )?;
    Ok(parse_ssh(&out, hours))
}

enum SshEvent<'a> {
    Failed {
        user: &'a str,
        ip: &'a str,
    },
    Accepted {
        user: &'a str,
        ip: &'a str,
        method: &'a str,
        key: &'a str,
    },
}

/// `<unix.micros> <host> sshd-session[pid]: <message>`
fn split_line(line: &str) -> Option<(i64, &str)> {
    let (ts, rest) = line.split_once(' ')?;
    let ts = ts.split('.').next()?.parse().ok()?;
    let (_, msg) = rest.split_once(": ")?;
    Some((ts, msg))
}

fn parse_event(msg: &str) -> Option<SshEvent<'_>> {
    if let Some(rest) = msg.strip_prefix("Failed ") {
        // Failed password for [invalid user ]X from IP port N ssh2
        let (_, rest) = rest.split_once(" for ")?;
        let rest = rest.strip_prefix("invalid user ").unwrap_or(rest);
        let (user, rest) = rest.rsplit_once(" from ")?;
        let ip = rest.split(' ').next()?;
        return Some(SshEvent::Failed { user, ip });
    }
    if let Some(rest) = msg.strip_prefix("Accepted ") {
        // Accepted publickey for root from IP port N ssh2: ED25519 SHA256:…
        let (method, rest) = rest.split_once(" for ")?;
        let (user, rest) = rest.split_once(" from ")?;
        let ip = rest.split(' ').next()?;
        let key = rest.split_once(": ").map(|(_, k)| k.trim()).unwrap_or("");
        return Some(SshEvent::Accepted {
            user,
            ip,
            method,
            key,
        });
    }
    None
}

fn parse_ssh(out: &str, hours: u32) -> SshActivity {
    let mut hourly: BTreeMap<i64, u64> = BTreeMap::new();
    let mut ips: HashMap<&str, SshIp> = HashMap::new();
    let mut users: HashMap<&str, u64> = HashMap::new();
    let mut accepted = Vec::new();
    let mut failed = 0u64;

    for line in out.lines() {
        let Some((ts, msg)) = split_line(line) else {
            continue;
        };
        match parse_event(msg) {
            Some(SshEvent::Failed { user, ip }) => {
                failed += 1;
                *hourly.entry(ts - ts.rem_euclid(3600)).or_default() += 1;
                *users.entry(user).or_default() += 1;
                let e = ips.entry(ip).or_insert_with(|| SshIp {
                    ip: ip.to_string(),
                    first: ts,
                    ..Default::default()
                });
                e.count += 1;
                e.last = ts;
                e.last_user = user.to_string();
            }
            Some(SshEvent::Accepted {
                user,
                ip,
                method,
                key,
            }) => accepted.push(SshLogin {
                ts,
                user: user.to_string(),
                ip: ip.to_string(),
                method: method.to_string(),
                key: key.to_string(),
            }),
            None => {}
        }
    }

    let unique_ips = ips.len() as u64;
    let mut top_ips: Vec<SshIp> = ips.into_values().collect();
    top_ips.sort_by(|a, b| b.count.cmp(&a.count).then(b.last.cmp(&a.last)));
    top_ips.truncate(50);
    let mut top_users: Vec<(String, u64)> =
        users.into_iter().map(|(u, n)| (u.to_string(), n)).collect();
    top_users.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    top_users.truncate(30);
    accepted.reverse();
    accepted.truncate(200);

    SshActivity {
        hours,
        failed,
        unique_ips,
        hourly: hourly.into_iter().collect(),
        top_ips,
        top_users,
        accepted,
    }
}

pub fn listening() -> Result<Vec<ListeningSocket>> {
    Ok(parse_ss(&run("ss", &["-tlnupH"])?))
}

fn parse_ss(out: &str) -> Vec<ListeningSocket> {
    let mut seen = HashSet::new();
    let mut res = Vec::new();
    for line in out.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 5 {
            continue;
        }
        let Some((addr, port)) = f[4].rsplit_once(':') else {
            continue;
        };
        let Ok(port) = port.parse::<u16>() else {
            continue;
        };
        let addr = addr.trim_start_matches('[').trim_end_matches(']');
        let addr = addr.split('%').next().unwrap_or(addr).to_string();
        let process = f
            .get(6)
            .and_then(|p| p.split("((\"").nth(1))
            .and_then(|p| p.split('"').next())
            .unwrap_or("")
            .to_string();
        let public = !(addr.starts_with("127.") || addr == "::1" || addr == "localhost");
        if seen.insert((f[0].to_string(), addr.clone(), port)) {
            res.push(ListeningSocket {
                proto: f[0].to_string(),
                address: addr,
                port,
                process,
                public,
            });
        }
    }
    res.sort_by(|a, b| (a.port, &a.proto, &a.address).cmp(&(b.port, &b.proto, &b.address)));
    res
}

pub fn firewall() -> Result<Vec<FirewallTable>> {
    Ok(parse_nft(&run("nft", &["list", "ruleset"])?))
}

fn parse_nft(out: &str) -> Vec<FirewallTable> {
    let mut res = Vec::new();
    let mut cur: Option<FirewallTable> = None;
    for line in out.lines() {
        if let Some(head) = line.strip_prefix("table ") {
            let mut it = head.trim_end_matches('{').split_whitespace();
            cur = Some(FirewallTable {
                family: it.next().unwrap_or_default().to_string(),
                name: it.next().unwrap_or_default().to_string(),
                body: String::new(),
            });
        } else if line == "}" {
            res.extend(cur.take());
        } else if let Some(t) = cur.as_mut() {
            t.body.push_str(line);
            t.body.push('\n');
        }
    }
    res
}

pub fn sshd_config() -> Result<BTreeMap<String, String>> {
    let out = run("sshd", &["-T"])?;
    Ok(out
        .lines()
        .filter_map(|l| l.split_once(' '))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect())
}

fn unit_active(unit: &str) -> bool {
    try_run("systemctl", &["is-active", unit]).is_some_and(|s| s.trim() == "active")
}

pub fn tools() -> SecurityTools {
    SecurityTools {
        fail2ban: unit_active("fail2ban.service"),
        crowdsec: unit_active("crowdsec.service"),
        ufw: try_run("ufw", &["status"]).map(|s| s.lines().next().unwrap_or("").to_string()),
    }
}

pub fn updates() -> Result<Updates> {
    let out = run("apt", &["list", "--upgradable"])?;
    let pkgs = std::fs::read_to_string("/var/run/reboot-required.pkgs").unwrap_or_default();
    let mut reboot_packages: Vec<String> = pkgs.lines().map(str::to_string).collect();
    reboot_packages.sort();
    reboot_packages.dedup();
    let stamp = [
        "/var/lib/apt/periodic/update-success-stamp",
        "/var/lib/apt/periodic/download-upgradeable-stamp",
        "/var/lib/apt/lists/partial",
    ]
    .iter()
    .find_map(|p| std::fs::metadata(p).ok()?.modified().ok())
    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
    .map(|d| d.as_secs() as i64);
    Ok(Updates {
        packages: parse_apt(&out),
        reboot_required: std::path::Path::new("/var/run/reboot-required").exists(),
        reboot_packages,
        lists_updated: stamp,
    })
}

fn parse_apt(out: &str) -> Vec<UpgradablePackage> {
    out.lines()
        .filter_map(|l| {
            // name/suite[,suite] candidate arch [upgradable from: current]
            let (name, rest) = l.split_once('/')?;
            let mut f = rest.split_whitespace();
            let suite = f.next()?.to_string();
            let candidate = f.next()?.to_string();
            let current = l
                .split("upgradable from: ")
                .nth(1)
                .map(|s| s.trim_end_matches(']').to_string())
                .unwrap_or_default();
            Some(UpgradablePackage {
                name: name.to_string(),
                security: suite.contains("-security"),
                suite,
                current,
                candidate,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const JOURNAL: &str = "\
1793718171.838527 my sshd-session[866720]: Accepted publickey for root from 198.51.100.7 port 14336 ssh2: ED25519 SHA256:Wf1T
1793718200.000001 my sshd-session[1]: Invalid user oracle from 203.0.113.9 port 4242
1793718201.000001 my sshd-session[1]: pam_unix(sshd:auth): check pass; user unknown
1793718202.000001 my sshd-session[1]: Failed password for invalid user oracle from 203.0.113.9 port 4242 ssh2
1793721800.000001 my sshd-session[2]: Failed password for root from 203.0.113.9 port 5555 ssh2
1793721801.000001 my sshd-session[3]: Failed password for root from 192.0.2.1 port 1 ssh2
1793721802.000001 my sshd-session[3]: Connection closed by authenticating user root 192.0.2.1 port 1 [preauth]
garbage line
";

    #[test]
    fn ssh() {
        let a = parse_ssh(JOURNAL, 24);
        assert_eq!(a.failed, 3);
        assert_eq!(a.unique_ips, 2);
        assert_eq!(a.top_ips[0].ip, "203.0.113.9");
        assert_eq!(a.top_ips[0].count, 2);
        assert_eq!(a.top_ips[0].last_user, "root");
        assert_eq!(a.top_users[0], ("root".to_string(), 2));
        assert_eq!(a.hourly.len(), 2);
        assert_eq!(a.hourly.iter().map(|h| h.1).sum::<u64>(), 3);
        assert_eq!(a.accepted.len(), 1);
        assert_eq!(a.accepted[0].method, "publickey");
        assert_eq!(a.accepted[0].ip, "198.51.100.7");
        assert_eq!(a.accepted[0].key, "ED25519 SHA256:Wf1T");
    }

    #[test]
    fn ss() {
        let out = r#"udp UNCONN 0      0                127.0.0.54:53    0.0.0.0:* users:(("systemd-resolve",pid=3910041,fd=19))
udp UNCONN 0      0             127.0.0.53%lo:53    0.0.0.0:* users:(("systemd-resolve",pid=3910041,fd=17))
udp UNCONN 0      0                   0.0.0.0:443   0.0.0.0:* users:(("docker-proxy",pid=850976,fd=8))
udp UNCONN 0      0                     [::1]:323      [::]:* users:(("chronyd",pid=1096,fd=5))
tcp LISTEN 0      4096                0.0.0.0:22    0.0.0.0:* users:(("sshd",pid=3910037,fd=3),("systemd",pid=1,fd=314))
tcp LISTEN 0      4096                   [::]:22       [::]:* users:(("sshd",pid=3910037,fd=4))
"#;
        let s = parse_ss(out);
        assert_eq!(s.len(), 6);
        let ssh = s
            .iter()
            .find(|x| x.port == 22 && x.address == "0.0.0.0")
            .unwrap();
        assert_eq!(ssh.process, "sshd");
        assert!(ssh.public);
        let dns = s.iter().find(|x| x.address == "127.0.0.53").unwrap();
        assert!(!dns.public);
        assert!(!s.iter().find(|x| x.address == "::1").unwrap().public);
        assert!(s.iter().find(|x| x.address == "::").unwrap().public);
        assert_eq!(
            s.iter().find(|x| x.port == 443).unwrap().process,
            "docker-proxy"
        );
    }

    #[test]
    fn nft() {
        let out = "# Warning: table ip nat is managed by iptables-nft, do not touch!\ntable ip nat {\n\tchain DOCKER {\n\t}\n}\ntable inet cf_origin {\n\tset cf6 {\n\t}\n}\n";
        let t = parse_nft(out);
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].family.as_str(), t[0].name.as_str()), ("ip", "nat"));
        assert!(t[0].body.contains("chain DOCKER"));
        assert_eq!(t[1].name, "cf_origin");
    }

    #[test]
    fn apt() {
        let out = "Listing...\ncloud-init/resolute-updates 26.2-0ubuntu1~26.04.1 all [upgradable from: 26.1-0ubuntu3~26.04.1]\nopenssl/resolute-security,resolute-updates 3.5.0-1ubuntu2 amd64 [upgradable from: 3.5.0-1ubuntu1]\ncontainerd.io/resolute 2.4.2-3~ubuntu.26.04~resolute amd64 [upgradable from: 2.3.5-1~ubuntu.26.04~resolute]\n";
        let p = parse_apt(out);
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].name, "cloud-init");
        assert_eq!(p[0].current, "26.1-0ubuntu3~26.04.1");
        assert_eq!(p[0].candidate, "26.2-0ubuntu1~26.04.1");
        assert!(!p[0].security);
        assert!(p[1].security);
        assert_eq!(p[2].name, "containerd.io");
    }
}
