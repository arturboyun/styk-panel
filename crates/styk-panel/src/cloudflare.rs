//! Just enough of the Cloudflare API to create a proxied A record for a new site.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

const API: &str = "https://api.cloudflare.com/client/v4";

async fn get(http: &reqwest::Client, token: &str, url: &str) -> Result<Value> {
    let v: Value = http
        .get(url)
        .bearer_auth(token)
        .send()
        .await?
        .json()
        .await?;
    if v["success"] != true {
        bail!("cloudflare: {}", v["errors"]);
    }
    Ok(v)
}

/// Finds the zone by walking up the name (`a.b.example.com` → `b.example.com` → …).
async fn zone_id(http: &reqwest::Client, token: &str, domain: &str) -> Result<String> {
    let labels: Vec<&str> = domain.trim_start_matches("*.").split('.').collect();
    for i in 0..labels.len().saturating_sub(1) {
        let name = labels[i..].join(".");
        let v = get(http, token, &format!("{API}/zones?name={name}")).await?;
        if let Some(id) = v["result"][0]["id"].as_str() {
            return Ok(id.to_string());
        }
    }
    bail!("zone for {domain} not found in this Cloudflare account")
}

/// Creates (or confirms) a proxied A record. Returns a human-readable outcome.
pub async fn ensure_a_record(
    http: &reqwest::Client,
    token: &str,
    domain: &str,
    ip: &str,
) -> Result<String> {
    let zone = zone_id(http, token, domain).await?;
    let existing = get(
        http,
        token,
        &format!("{API}/zones/{zone}/dns_records?name={domain}"),
    )
    .await?;
    if let Some(r) = existing["result"].as_array().and_then(|a| a.first()) {
        return Ok(format!(
            "DNS: запись уже есть ({} {})",
            r["type"].as_str().unwrap_or("?"),
            r["content"].as_str().unwrap_or("?")
        ));
    }
    let v: Value = http
        .post(format!("{API}/zones/{zone}/dns_records"))
        .bearer_auth(token)
        .json(
            &json!({ "type": "A", "name": domain, "content": ip, "proxied": true, "ttl": 1,
                       "comment": "created by styk-panel" }),
        )
        .send()
        .await
        .context("cloudflare request")?
        .json()
        .await?;
    if v["success"] != true {
        bail!("cloudflare: {}", v["errors"]);
    }
    Ok(format!("DNS: создана A-запись {domain} → {ip} (proxied)"))
}
