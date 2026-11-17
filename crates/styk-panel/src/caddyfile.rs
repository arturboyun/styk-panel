//! Text-level Caddyfile model: enough to list sites and to add/remove the blocks the
//! panel manages, without ever rewriting hand-written parts of the file.
//!
//! Relies on two Caddyfile rules: an opening brace is the last token of its line and a
//! closing brace stands on its own line.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub const MARK_BEGIN: &str = "# >>> styk-panel: ";
pub const MARK_END: &str = "# <<< styk-panel";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Global,
    Snippet,
    Site,
}

#[derive(Debug, Clone, Serialize)]
pub struct Block {
    pub kind: Kind,
    pub header: String,
    /// First and last line (0-based, inclusive) of the block itself.
    pub start: usize,
    pub end: usize,
    /// Managed blocks: line range including the markers.
    pub managed: Option<(usize, usize)>,
    pub comment: Vec<String>,
    pub body: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Site {
    pub hosts: Vec<String>,
    pub raw_hosts: Vec<String>,
    pub upstreams: Vec<String>,
    pub cloudflare_only: bool,
    /// http (HTTP-01/ALPN) | dns | on_demand | internal
    pub tls: String,
    pub redirect: Option<String>,
    pub static_files: bool,
    pub managed: bool,
    pub comment: String,
    pub line: usize,
}

fn opens(line: &str) -> bool {
    let t = strip_comment(line).trim_end();
    t == "{" || t.ends_with(" {") || t.ends_with("\t{")
}

fn closes(line: &str) -> bool {
    strip_comment(line).trim() == "}"
}

/// `#` starts a comment only at the beginning of a token.
fn strip_comment(line: &str) -> &str {
    let b = line.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        if c == b'#' && (i == 0 || b[i - 1].is_ascii_whitespace()) {
            return &line[..i];
        }
    }
    line
}

pub fn parse(text: &str) -> Vec<Block> {
    let lines: Vec<&str> = text.lines().collect();
    let mut blocks = Vec::new();
    let mut depth = 0usize;
    let mut comment: Vec<String> = Vec::new();
    let mut managed_start: Option<usize> = None;
    let mut cur: Option<Block> = None;

    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if depth == 0 {
            if let Some(rest) = t.strip_prefix(MARK_BEGIN.trim_end()) {
                let _ = rest;
                managed_start = Some(i);
                comment.clear();
                continue;
            }
            if t.starts_with(MARK_END) {
                if let (Some(s), Some(b)) = (managed_start.take(), blocks.last_mut()) {
                    let b: &mut Block = b;
                    if b.start > s {
                        b.managed = Some((s, i));
                    }
                }
                continue;
            }
            if t.is_empty() {
                comment.clear();
                continue;
            }
            if t.starts_with('#') {
                comment.push(t.trim_start_matches('#').trim().to_string());
                continue;
            }
            if opens(line) {
                let header = strip_comment(line)
                    .trim_end()
                    .trim_end_matches('{')
                    .trim()
                    .to_string();
                let kind = if header.is_empty() {
                    Kind::Global
                } else if header.starts_with('(') {
                    Kind::Snippet
                } else {
                    Kind::Site
                };
                cur = Some(Block {
                    kind,
                    header,
                    start: i,
                    end: i,
                    managed: None,
                    comment: std::mem::take(&mut comment),
                    body: Vec::new(),
                });
                depth = 1;
                continue;
            }
            // A site without a block (`example.com` followed by directives on one line)
            // is legal but unused here; treat it as an opaque line.
            comment.clear();
            continue;
        }
        if closes(line) {
            depth -= 1;
            if depth == 0 {
                if let Some(mut b) = cur.take() {
                    b.end = i;
                    blocks.push(b);
                }
                continue;
            }
        } else if opens(line) {
            depth += 1;
        }
        if let Some(b) = cur.as_mut() {
            b.body.push(line.to_string());
        }
    }
    blocks
}

/// `{$VAR}` and `{$VAR:default}` from the Caddy container's environment.
pub fn resolve(s: &str, env: &HashMap<String, String>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("{$") {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('}') else {
            out.push_str(&rest[i..]);
            return out;
        };
        let inner = &rest[i + 2..i + j];
        let (name, default) = inner.split_once(':').unwrap_or((inner, ""));
        out.push_str(env.get(name).map(String::as_str).unwrap_or(default));
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    out
}

fn looks_like_upstream(tok: &str) -> bool {
    !tok.starts_with('@')
        && !tok.starts_with('/')
        && !tok.starts_with('*')
        && tok != "{"
        && (tok.contains(':') || tok.starts_with("http"))
}

pub fn sites(blocks: &[Block], env: &HashMap<String, String>) -> Vec<Site> {
    blocks
        .iter()
        .filter(|b| b.kind == Kind::Site)
        .map(|b| {
            let raw_hosts: Vec<String> = b
                .header
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            let hosts = raw_hosts.iter().map(|h| resolve(h, env)).collect();
            let mut s = Site {
                hosts,
                raw_hosts,
                upstreams: Vec::new(),
                cloudflare_only: false,
                tls: "http".into(),
                redirect: None,
                static_files: false,
                managed: b.managed.is_some(),
                comment: b.comment.join(" "),
                line: b.start + 1,
            };
            let mut in_tls = false;
            for line in &b.body {
                let toks: Vec<&str> = strip_comment(line).split_whitespace().collect();
                let Some(&first) = toks.first() else { continue };
                match first {
                    "reverse_proxy" => {
                        for t in toks[1..].iter().filter(|t| looks_like_upstream(t)) {
                            let t = resolve(t, env);
                            if !s.upstreams.contains(&t) {
                                s.upstreams.push(t);
                            }
                        }
                    }
                    "to" if toks.len() > 1 => {
                        for t in &toks[1..] {
                            s.upstreams.push(resolve(t, env));
                        }
                    }
                    "import" if toks.get(1) == Some(&"cloudflare_only") => s.cloudflare_only = true,
                    "tls" => {
                        if toks.get(1) == Some(&"internal") {
                            s.tls = "internal".into();
                        }
                        in_tls = toks.last() == Some(&"{");
                    }
                    "dns" if in_tls => s.tls = "dns".into(),
                    "on_demand" if in_tls => s.tls = "on_demand".into(),
                    "}" => in_tls = false,
                    "redir" => {
                        s.redirect = toks
                            .iter()
                            .skip(1)
                            .find(|t| t.contains("://"))
                            .map(|t| t.to_string())
                    }
                    "file_server" => s.static_files = true,
                    _ => {}
                }
            }
            s
        })
        .collect()
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewSite {
    pub domain: String,
    pub upstream: String,
    #[serde(default = "yes")]
    pub cloudflare_only: bool,
    #[serde(default = "yes")]
    pub dns_challenge: bool,
    #[serde(default = "yes")]
    pub compress: bool,
    #[serde(default = "yes")]
    pub hsts: bool,
}

fn yes() -> bool {
    true
}

pub fn valid_domain(d: &str) -> bool {
    let labels: Vec<&str> = d.split('.').collect();
    d.len() <= 253
        && labels.len() >= 2
        && labels.iter().enumerate().all(|(i, l)| {
            (i == 0 && *l == "*")
                || (!l.is_empty()
                    && l.len() <= 63
                    && !l.starts_with('-')
                    && !l.ends_with('-')
                    && l.chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
        })
}

pub fn valid_upstream(u: &str) -> bool {
    let hostport = u
        .strip_prefix("http://")
        .or_else(|| u.strip_prefix("https://"))
        .unwrap_or(u);
    let Some((host, port)) = hostport.rsplit_once(':') else {
        return false;
    };
    !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        && port.parse::<u16>().is_ok_and(|p| p > 0)
}

pub fn render(s: &NewSite) -> String {
    let mut b = format!("{MARK_BEGIN}{}\n{} {{\n", s.domain, s.domain);
    if s.cloudflare_only {
        b.push_str("\timport cloudflare_only\n");
    }
    if s.dns_challenge {
        b.push_str("\ttls {\n\t\tdns cloudflare {env.CF_API_TOKEN}\n\t}\n");
    }
    if s.compress {
        b.push_str("\tencode zstd gzip\n");
    }
    b.push_str("\theader -Server\n");
    if s.hsts {
        b.push_str("\theader Strict-Transport-Security \"max-age=31536000\"\n");
    }
    b.push_str(&format!("\treverse_proxy {}\n}}\n{MARK_END}\n", s.upstream));
    b
}

/// The file with a managed block appended, or an error when the domain already exists.
pub fn add_site(
    text: &str,
    site: &NewSite,
    env: &HashMap<String, String>,
) -> Result<String, String> {
    let blocks = parse(text);
    if sites(&blocks, env)
        .iter()
        .any(|s| s.hosts.iter().any(|h| h == &site.domain))
    {
        return Err(format!("{} уже есть в Caddyfile", site.domain));
    }
    let mut out = text.trim_end().to_string();
    out.push_str("\n\n");
    out.push_str(&render(site));
    Ok(out)
}

pub fn remove_site(text: &str, domain: &str) -> Result<String, String> {
    let blocks = parse(text);
    let b = blocks
        .iter()
        .find(|b| b.managed.is_some() && b.header == domain)
        .ok_or_else(|| format!("{domain}: нет управляемого панелью блока"))?;
    let (s, e) = b.managed.unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<&str> = lines[..s].to_vec();
    // Drop the blank line that separated the block from the previous one.
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    let tail = &lines[e + 1..];
    if !tail.is_empty() && !out.is_empty() {
        out.push("");
    }
    out.extend(tail.iter().skip_while(|l| l.trim().is_empty()));
    let mut s = out.join("\n");
    s.push('\n');
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"{
	on_demand_tls {
		ask http://tunnel:8080/_ask
	}
}

(cloudflare_only) {
	@not_cloudflare not remote_ip private_ranges 173.245.48.0/20
	abort @not_cloudflare
}

{$APP_DOMAIN} {
	import cloudflare_only
	encode zstd gzip
	reverse_proxy bot:8080
	header Strict-Transport-Security "max-age=31536000"
}

{$WEBHOOK_DOMAIN} {
	handle /telegram/webhook {
		reverse_proxy bot:8080
	}
	handle {
		respond 404
	}
}

# Company site
styk.example.org, example.org {
	import cloudflare_only
	tls {
		dns cloudflare {env.CF_API_TOKEN}
	}
	reverse_proxy site-web-1:80
}

www.example.org {
	redir https://example.org{uri} permanent
}

https:// {
	tls {
		on_demand
	}
	reverse_proxy tunnel:8080
}
"#;

    fn env() -> HashMap<String, String> {
        [("APP_DOMAIN", "app.example.org")]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn parses_sites() {
        let b = parse(FILE);
        assert_eq!(b.iter().filter(|b| b.kind == Kind::Global).count(), 1);
        assert_eq!(b.iter().filter(|b| b.kind == Kind::Snippet).count(), 1);
        let s = sites(&b, &env());
        assert_eq!(s.len(), 5);
        assert_eq!(s[0].hosts, vec!["app.example.org"]);
        assert!(s[0].cloudflare_only);
        assert_eq!(s[0].upstreams, vec!["bot:8080"]);
        assert_eq!(s[1].hosts, vec![""], "unset variable resolves to empty");
        assert_eq!(s[1].upstreams, vec!["bot:8080"]);
        assert_eq!(s[2].hosts, vec!["styk.example.org", "example.org"]);
        assert_eq!(s[2].tls, "dns");
        assert_eq!(s[2].comment, "Company site");
        assert_eq!(s[3].redirect.as_deref(), Some("https://example.org{uri}"));
        assert_eq!(s[4].tls, "on_demand");
        assert!(!s.iter().any(|s| s.managed));
    }

    #[test]
    fn add_and_remove() {
        let site = NewSite {
            domain: "new.example.org".into(),
            upstream: "app:3000".into(),
            cloudflare_only: true,
            dns_challenge: true,
            compress: true,
            hsts: true,
        };
        let added = add_site(FILE, &site, &env()).unwrap();
        let s = sites(&parse(&added), &env());
        let n = s.iter().find(|s| s.hosts == ["new.example.org"]).unwrap();
        assert!(n.managed && n.cloudflare_only);
        assert_eq!(n.tls, "dns");
        assert_eq!(n.upstreams, ["app:3000"]);
        assert!(add_site(&added, &site, &env()).is_err(), "duplicate");
        let dup = NewSite {
            domain: "app.example.org".into(),
            ..site.clone()
        };
        assert!(
            add_site(FILE, &dup, &env()).is_err(),
            "resolved placeholder"
        );

        let removed = remove_site(&added, "new.example.org").unwrap();
        assert_eq!(removed.trim_end(), FILE.trim_end());
        assert!(
            remove_site(FILE, "www.example.org").is_err(),
            "hand-written"
        );
    }

    #[test]
    fn validation() {
        assert!(valid_domain("a.example.org"));
        assert!(valid_domain("*.tunnel.example.org"));
        assert!(!valid_domain("example"));
        assert!(!valid_domain("a b.com"));
        assert!(!valid_domain("x.com {\n}"));
        assert!(valid_upstream("app:3000"));
        assert!(valid_upstream("http://10.0.0.1:80"));
        assert!(!valid_upstream("app"));
        assert!(!valid_upstream("app:3000\n}"));
        assert_eq!(resolve("{$X:def}.com", &HashMap::new()), "def.com");
    }
}
