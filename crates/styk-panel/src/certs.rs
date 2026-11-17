//! Expiry dates of the certificates Caddy keeps in its data volume. A minimal DER walk
//! (Certificate → tbsCertificate → validity.notAfter) instead of a full X.509 parser.

use base64::Engine;
use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertInfo {
    pub domain: String,
    pub issuer: String,
    pub not_after: i64,
}

fn tlv(d: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let tag = *d.first()?;
    let l0 = *d.get(1)? as usize;
    let (len, hdr) = if l0 < 0x80 {
        (l0, 2)
    } else {
        let n = l0 & 0x7f;
        if n == 0 || n > 4 {
            return None;
        }
        let mut len = 0usize;
        for i in 0..n {
            len = (len << 8) | *d.get(2 + i)? as usize;
        }
        (len, 2 + n)
    };
    let body = d.get(hdr..hdr + len)?;
    Some((tag, body, &d[hdr + len..]))
}

fn parse_time(tag: u8, s: &[u8]) -> Option<i64> {
    let s = std::str::from_utf8(s).ok()?.trim_end_matches('Z');
    let (year, rest) = match tag {
        0x17 => {
            let yy: i32 = s.get(..2)?.parse().ok()?;
            (if yy < 50 { 2000 + yy } else { 1900 + yy }, s.get(2..)?)
        }
        0x18 => (s.get(..4)?.parse().ok()?, s.get(4..)?),
        _ => return None,
    };
    let n = |a: usize| -> Option<u32> { rest.get(a..a + 2)?.parse().ok() };
    let dt: NaiveDateTime = NaiveDate::from_ymd_opt(year, n(0)?, n(2)?)?.and_hms_opt(
        n(4)?,
        n(6)?,
        n(8).unwrap_or(0),
    )?;
    Some(Utc.from_utc_datetime(&dt).timestamp())
}

pub fn not_after(der: &[u8]) -> Option<i64> {
    let (_, cert, _) = tlv(der)?;
    let (_, tbs, _) = tlv(cert)?;
    let mut rest = tbs;
    let (tag, _, r) = tlv(rest)?;
    if tag == 0xa0 {
        rest = r; // explicit version
    }
    let (_, _, rest) = tlv(rest)?; // serial
    let (_, _, rest) = tlv(rest)?; // signature algorithm
    let (_, _, rest) = tlv(rest)?; // issuer
    let (_, validity, _) = tlv(rest)?;
    let (_, _, v) = tlv(validity)?; // notBefore
    let (tag, t, _) = tlv(v)?;
    parse_time(tag, t)
}

/// Input: `==> <path>` lines each followed by the PEM file contents.
pub fn parse_dump(out: &str) -> Vec<CertInfo> {
    let mut res = Vec::new();
    for part in out.split("==> ").skip(1) {
        let Some((path, body)) = part.split_once('\n') else {
            continue;
        };
        let Some(b64) = body
            .split("-----BEGIN CERTIFICATE-----")
            .nth(1)
            .and_then(|s| s.split("-----END CERTIFICATE-----").next())
        else {
            continue;
        };
        let clean: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
        let Ok(der) = base64::engine::general_purpose::STANDARD.decode(clean) else {
            continue;
        };
        let Some(not_after) = not_after(&der) else {
            continue;
        };
        let mut it = path.trim().rsplit('/');
        let file = it.next().unwrap_or_default();
        let _dir = it.next();
        let issuer_dir = it.next().unwrap_or_default();
        let domain = file.trim_end_matches(".crt").replacen("wildcard_", "*", 1);
        let issuer = if issuer_dir.contains("letsencrypt") {
            "Let's Encrypt".to_string()
        } else if issuer_dir.contains("zerossl") {
            "ZeroSSL".to_string()
        } else if issuer_dir == "local" {
            "Caddy local CA".to_string()
        } else {
            issuer_dir.to_string()
        };
        res.push(CertInfo {
            domain,
            issuer,
            not_after,
        });
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(parse_time(0x17, b"260201120000Z"), Some(1769947200));
        assert_eq!(parse_time(0x18, b"20500101000000Z"), Some(2524608000));
    }

    #[test]
    fn der_walk() {
        // SEQ{ SEQ{ [0]{INT 2}, INT 1, SEQ{}, SEQ{}, SEQ{ UTC, UTC } } }
        let nb = b"250101000000Z";
        let na = b"260201120000Z";
        let mut validity = vec![0x17, 13];
        validity.extend_from_slice(nb);
        validity.extend([0x17, 13]);
        validity.extend_from_slice(na);
        let mut tbs = vec![
            0xa0,
            3,
            0x02,
            1,
            2,
            0x02,
            1,
            1,
            0x30,
            0,
            0x30,
            0,
            0x30,
            validity.len() as u8,
        ];
        tbs.extend(validity);
        let mut cert = vec![0x30, tbs.len() as u8];
        cert.extend(tbs);
        let mut der = vec![0x30, cert.len() as u8];
        der.extend(cert);
        assert_eq!(not_after(&der), Some(1769947200));
        assert_eq!(not_after(&der[..10]), None);
    }
}
