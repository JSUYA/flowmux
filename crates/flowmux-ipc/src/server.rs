//! Skeleton daemon that accepts client connections and dispatches
//! requests via a user-supplied [`Handler`]. The GUI binary owns the
//! handler implementation; this crate only owns the wire protocol.

use crate::protocol::{Envelope, Payload, Request, Response, RpcError};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::{error, info, warn};

pub trait Handler: Send + Sync + 'static {
    fn handle<'a>(
        &'a self,
        req: Request,
    ) -> Pin<Box<dyn Future<Output = Response> + Send + 'a>>;
}

pub async fn run<H: Handler>(socket: &Path, handler: Arc<H>) -> anyhow::Result<()> {
    if socket.exists() {
        std::fs::remove_file(socket)?;
    }
    let listener = UnixListener::bind(socket)?;
    info!(path = %socket.display(), "flowmux daemon listening");
    loop {
        let (stream, _) = listener.accept().await?;
        let h = handler.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_one(stream, h).await {
                warn!(error = %e, "client disconnected with error");
            }
        });
    }
}

async fn serve_one<H: Handler>(stream: UnixStream, handler: Arc<H>) -> anyhow::Result<()> {
    let (r, mut w) = stream.into_split();
    let mut reader = BufReader::new(r);
    let mut buf = String::new();
    loop {
        buf.clear();
        let n = reader.read_line(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }
        let env: Envelope = match serde_json::from_str(buf.trim_end()) {
            Ok(e) => e,
            Err(e) => {
                error!(error = %e, raw = %buf, "malformed envelope");
                continue;
            }
        };
        let response = match env.payload {
            Payload::Request(req) => handler.handle(req).await,
            Payload::Response(_) | Payload::Event(_) => {
                Response::Error(RpcError::InvalidArgument(
                    "client sent non-request payload".into(),
                ))
            }
        };
        let out = Envelope { id: env.id, payload: Payload::Response(response) };
        let mut line = serde_json::to_string(&out)?;
        line.push('\n');
        w.write_all(line.as_bytes()).await?;
        w.flush().await?;
    }
}
