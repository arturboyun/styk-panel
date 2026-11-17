//! File manager over the agent's sandboxed fs methods. Writes need sudo; so does reading
//! anything that looks like a secret (env files, keys, database dumps).

use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::{header, HeaderValue};
use axum::response::Response;
use axum::Json;
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use styk_proto::{FsChunk, FsEntry, FsListing, FsVersion, Method};

use crate::audit;
use crate::auth::{AuthUser, ClientInfo};
use crate::error::{agent_error, ApiError, ApiResult};
use crate::state::AppState;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;
const CHUNK: u32 = 1 << 20;
const EDIT_MAX: u64 = 2 << 20;

pub fn is_secret(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    let p = path.to_ascii_lowercase();
    name.starts_with(".env")
        || name.ends_with(".env")
        || [
            ".key", ".pem", ".p12", ".pfx", ".dump", ".sql", ".sql.gz", ".tar.gz", ".kdbx",
        ]
        .iter()
        .any(|e| name.ends_with(e))
        || [
            "id_rsa",
            "id_ed25519",
            "id_ecdsa",
            ".htpasswd",
            "credentials",
            "secrets.json",
        ]
        .iter()
        .any(|n| name.starts_with(n))
        || p.contains("/secrets/")
        || p.starts_with("/var/backups/")
        || p.contains("-backups/")
        || p.starts_with("/srv/backup-inbox/")
}

fn guard_secret(u: &AuthUser, path: &str) -> ApiResult<()> {
    if is_secret(path) {
        u.require_sudo()?;
    }
    Ok(())
}

async fn call<T: serde::de::DeserializeOwned>(st: &AppState, m: Method) -> ApiResult<T> {
    st.agent
        .call_timeout(m, Duration::from_secs(30))
        .await
        .map_err(agent_error)
}

pub async fn roots(State(st): State<AppState>, _u: AuthUser) -> Json<Vec<String>> {
    Json(st.cfg.fs_roots.clone())
}

#[derive(Deserialize)]
pub struct PathQuery {
    path: String,
}

pub async fn list(
    State(st): State<AppState>,
    _u: AuthUser,
    Query(q): Query<PathQuery>,
) -> ApiResult<Json<FsListing>> {
    Ok(Json(call(&st, Method::FsList { path: q.path }).await?))
}

pub async fn read(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Query(q): Query<PathQuery>,
) -> ApiResult<Json<Value>> {
    guard_secret(&u, &q.path)?;
    let stat: FsEntry = call(
        &st,
        Method::FsStat {
            path: q.path.clone(),
        },
    )
    .await?;
    if stat.kind != "file" && stat.kind != "symlink" {
        return Err(ApiError::BadRequest("это не файл".into()));
    }
    if stat.size > EDIT_MAX {
        return Ok(Json(json!({ "entry": stat, "too_large": true })));
    }
    let mut data = Vec::new();
    let mut offset = 0u64;
    loop {
        let c: FsChunk = call(
            &st,
            Method::FsRead {
                path: q.path.clone(),
                offset,
                len: CHUNK,
            },
        )
        .await?;
        let bytes = B64.decode(c.data_b64).map_err(anyhow::Error::from)?;
        offset += bytes.len() as u64;
        data.extend(bytes);
        if c.eof || offset > EDIT_MAX {
            break;
        }
    }
    if is_secret(&q.path) {
        audit::user_action(
            &st,
            &u,
            &client,
            "file.read_secret",
            &q.path,
            true,
            json!({}),
        )
        .await;
    }
    let binary = data.iter().take(8000).any(|&b| b == 0) || std::str::from_utf8(&data).is_err();
    Ok(Json(json!({
        "entry": stat,
        "binary": binary,
        "content": if binary { None } else { Some(String::from_utf8_lossy(&data)) },
    })))
}

pub async fn download(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Query(q): Query<PathQuery>,
) -> ApiResult<Response> {
    guard_secret(&u, &q.path)?;
    let stat: FsEntry = call(
        &st,
        Method::FsStat {
            path: q.path.clone(),
        },
    )
    .await?;
    audit::user_action(
        &st,
        &u,
        &client,
        "file.download",
        &q.path,
        true,
        json!({ "size": stat.size }),
    )
    .await;
    let path = q.path.clone();
    let stream = futures::stream::unfold(Some(0u64), move |state| {
        let st = st.clone();
        let path = path.clone();
        async move {
            let offset = state?;
            let res: ApiResult<FsChunk> = call(
                &st,
                Method::FsRead {
                    path,
                    offset,
                    len: CHUNK,
                },
            )
            .await;
            match res.map(|c| (B64.decode(&c.data_b64), c.eof)) {
                Ok((Ok(bytes), eof)) => {
                    let next = (!eof && !bytes.is_empty()).then_some(offset + bytes.len() as u64);
                    Some((Ok::<_, std::io::Error>(Bytes::from(bytes)), next))
                }
                _ => Some((Err(std::io::Error::other("read failed")), None)),
            }
        }
    });
    let name = stat.name.replace(['"', '\\', '\n', '\r'], "_");
    let mut resp = Response::new(Body::from_stream(stream));
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(stat.size));
    if let Ok(v) = HeaderValue::from_str(&format!(
        "attachment; filename=\"{}\"; filename*=UTF-8''{}",
        name.chars().filter(char::is_ascii).collect::<String>(),
        urlencode(&stat.name)
    )) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(resp)
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[derive(Deserialize)]
pub struct WriteReq {
    path: String,
    content: String,
    expected_mtime: Option<i64>,
}

pub async fn write(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Json(req): Json<WriteReq>,
) -> ApiResult<Json<FsEntry>> {
    u.require_sudo()?;
    // Always in place: keeps inodes, hard links and single-file bind mounts intact.
    let res: ApiResult<FsEntry> = call(
        &st,
        Method::FsWrite {
            path: req.path.clone(),
            content_b64: B64.encode(req.content.as_bytes()),
            in_place: true,
            expected_mtime: req.expected_mtime,
        },
    )
    .await;
    audit::user_action(
        &st,
        &u,
        &client,
        "file.write",
        &req.path,
        res.is_ok(),
        json!({ "size": req.content.len() }),
    )
    .await;
    Ok(Json(res?))
}

#[derive(Deserialize)]
pub struct UploadQuery {
    path: String,
    offset: u64,
    #[serde(default)]
    last: bool,
}

pub async fn upload(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Query(q): Query<UploadQuery>,
    body: Bytes,
) -> ApiResult<Json<FsEntry>> {
    u.require_sudo()?;
    let res: ApiResult<FsEntry> = call(
        &st,
        Method::FsUpload {
            path: q.path.clone(),
            offset: q.offset,
            data_b64: B64.encode(&body),
            last: q.last,
        },
    )
    .await;
    if q.last || res.is_err() {
        let size = q.offset + body.len() as u64;
        audit::user_action(
            &st,
            &u,
            &client,
            "file.upload",
            &q.path,
            res.is_ok(),
            json!({ "size": size }),
        )
        .await;
    }
    Ok(Json(res?))
}

pub async fn mkdir(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Json(req): Json<PathQuery>,
) -> ApiResult<Json<FsEntry>> {
    u.require_sudo()?;
    let res: ApiResult<FsEntry> = call(
        &st,
        Method::FsMkdir {
            path: req.path.clone(),
        },
    )
    .await;
    audit::user_action(
        &st,
        &u,
        &client,
        "file.mkdir",
        &req.path,
        res.is_ok(),
        json!({}),
    )
    .await;
    Ok(Json(res?))
}

#[derive(Deserialize)]
pub struct RenameReq {
    from: String,
    to: String,
}

pub async fn rename(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Json(req): Json<RenameReq>,
) -> ApiResult<Json<FsEntry>> {
    u.require_sudo()?;
    let res: ApiResult<FsEntry> = call(
        &st,
        Method::FsRename {
            from: req.from.clone(),
            to: req.to.clone(),
        },
    )
    .await;
    audit::user_action(
        &st,
        &u,
        &client,
        "file.rename",
        &req.from,
        res.is_ok(),
        json!({ "to": req.to }),
    )
    .await;
    Ok(Json(res?))
}

pub async fn trash(
    State(st): State<AppState>,
    u: AuthUser,
    client: ClientInfo,
    Json(req): Json<PathQuery>,
) -> ApiResult<Json<Value>> {
    u.require_sudo()?;
    let res: ApiResult<String> = call(
        &st,
        Method::FsTrash {
            path: req.path.clone(),
        },
    )
    .await;
    audit::user_action(
        &st,
        &u,
        &client,
        "file.trash",
        &req.path,
        res.is_ok(),
        json!({}),
    )
    .await;
    Ok(Json(json!({ "trash": res? })))
}

pub async fn history(
    State(st): State<AppState>,
    _u: AuthUser,
    Query(q): Query<PathQuery>,
) -> ApiResult<Json<Vec<FsVersion>>> {
    Ok(Json(call(&st, Method::FsHistory { path: q.path }).await?))
}

#[derive(Deserialize)]
pub struct VersionQuery {
    path: String,
    version: String,
}

/// Content of a saved version; it must belong to `path`'s history.
pub async fn version(
    State(st): State<AppState>,
    u: AuthUser,
    Query(q): Query<VersionQuery>,
) -> ApiResult<Json<Value>> {
    guard_secret(&u, &q.path)?;
    let versions: Vec<FsVersion> = call(
        &st,
        Method::FsHistory {
            path: q.path.clone(),
        },
    )
    .await?;
    if !versions.iter().any(|v| v.path == q.version) {
        return Err(ApiError::NotFound);
    }
    let c: FsChunk = call(
        &st,
        Method::FsRead {
            path: q.version,
            offset: 0,
            len: CHUNK,
        },
    )
    .await?;
    let bytes = B64.decode(c.data_b64).map_err(anyhow::Error::from)?;
    Ok(Json(json!({ "content": String::from_utf8_lossy(&bytes) })))
}

#[cfg(test)]
mod tests {
    use super::is_secret;

    #[test]
    fn secrets() {
        assert!(is_secret("/opt/app/.env"));
        assert!(is_secret("/opt/app/.env.production"));
        assert!(is_secret("/opt/app/prod.env"));
        assert!(is_secret("/opt/app/certs/tls.key"));
        assert!(is_secret("/var/backups/containers/db/db.dump"));
        assert!(is_secret("/opt/app/secrets/token"));
        assert!(!is_secret("/opt/app/compose.yml"));
        assert!(!is_secret("/opt/edge/caddy/Caddyfile"));
    }
}
