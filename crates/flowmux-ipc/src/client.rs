use crate::protocol::{Envelope, Payload, Request, Response};
use anyhow::{anyhow, Context};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::Mutex;

pub struct Client {
    inner: Mutex<Inner>,
    next_id: std::sync::atomic::AtomicU64,
}

struct Inner {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
}

impl Client {
    pub async fn connect(socket: &Path) -> anyhow::Result<Self> {
        let stream = UnixStream::connect(socket)
            .await
            .with_context(|| format!("connecting to flowmux socket at {}", socket.display()))?;
        let (r, w) = stream.into_split();
        Ok(Self {
            inner: Mutex::new(Inner { reader: BufReader::new(r), writer: w }),
            next_id: std::sync::atomic::AtomicU64::new(1),
        })
    }

    pub async fn call(&self, req: Request) -> anyhow::Result<Response> {
        let id = self.next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let env = Envelope { id, payload: Payload::Request(req) };
        let mut line = serde_json::to_string(&env)?;
        line.push('\n');

        let mut inner = self.inner.lock().await;
        inner.writer.write_all(line.as_bytes()).await?;
        inner.writer.flush().await?;

        let mut buf = String::new();
        loop {
            buf.clear();
            let n = inner.reader.read_line(&mut buf).await?;
            if n == 0 {
                return Err(anyhow!("daemon closed the connection"));
            }
            let env: Envelope = serde_json::from_str(buf.trim_end())?;
            if env.id != id {
                continue; // out-of-order event; ignore
            }
            match env.payload {
                Payload::Response(r) => return Ok(r),
                Payload::Event(_) | Payload::Request(_) => continue,
            }
        }
    }
}
