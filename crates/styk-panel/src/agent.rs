//! Client for styk-agent over its unix socket: a small pool of connections, so a slow
//! call (`apt list`, journal scans) never holds up metrics collection.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;
use styk_proto::{Method, Reply, ReplyResult, Request};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;
use tokio::sync::Mutex;

pub const TIMEOUT: Duration = Duration::from_secs(10);
const POOL: usize = 8;

type Conn = (BufReader<OwnedReadHalf>, OwnedWriteHalf);

pub struct AgentClient {
    path: PathBuf,
    conns: Vec<Mutex<Option<Conn>>>,
    next_conn: AtomicUsize,
    next_id: AtomicU64,
}

impl AgentClient {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            conns: (0..POOL).map(|_| Mutex::new(None)).collect(),
            next_conn: AtomicUsize::new(0),
            next_id: AtomicU64::new(1),
        }
    }

    pub async fn call<T: DeserializeOwned>(&self, method: Method) -> Result<T> {
        self.call_timeout(method, TIMEOUT).await
    }

    pub async fn call_timeout<T: DeserializeOwned>(
        &self,
        method: Method,
        timeout: Duration,
    ) -> Result<T> {
        // A free connection if there is one, otherwise queue on the next in turn.
        let mut guard = match self.conns.iter().find_map(|c| c.try_lock().ok()) {
            Some(g) => g,
            None => {
                let i = self.next_conn.fetch_add(1, Ordering::Relaxed) % POOL;
                self.conns[i].lock().await
            }
        };
        let res = tokio::time::timeout(timeout, self.roundtrip(&mut guard, method)).await;
        let value = match res {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                // Agent-side errors leave the connection usable; I/O errors do not.
                if !e.to_string().starts_with("styk-agent: ") {
                    *guard = None;
                }
                return Err(e);
            }
            Err(_) => {
                *guard = None;
                bail!("styk-agent did not answer in {timeout:?}");
            }
        };
        Ok(serde_json::from_value(value)?)
    }

    async fn roundtrip(
        &self,
        conn: &mut Option<Conn>,
        method: Method,
    ) -> Result<serde_json::Value> {
        if conn.is_none() {
            let s = UnixStream::connect(&self.path)
                .await
                .with_context(|| format!("connect to styk-agent at {}", self.path.display()))?;
            let (r, w) = s.into_split();
            *conn = Some((BufReader::new(r), w));
        }
        let (rd, wr) = conn.as_mut().unwrap();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut buf = serde_json::to_vec(&Request { id, method })?;
        buf.push(b'\n');
        wr.write_all(&buf).await?;
        let mut line = String::new();
        if rd.read_line(&mut line).await? == 0 {
            bail!("styk-agent closed the connection");
        }
        let reply: Reply = serde_json::from_str(&line)?;
        if reply.id != id {
            bail!("styk-agent reply id mismatch");
        }
        match reply.result {
            ReplyResult::Ok(v) => Ok(v),
            ReplyResult::Err(e) => bail!("styk-agent: {e}"),
        }
    }
}
