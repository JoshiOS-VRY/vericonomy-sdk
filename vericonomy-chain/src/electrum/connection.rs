//! TCP/TLS connection to an Electrum server with newline-framed JSON-RPC.

use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use rustls::pki_types::ServerName;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};
use tokio_rustls::TlsConnector;

use super::protocol::{ElectrumRequest, ElectrumResponse};
use vericonomy_errors::{Result, WalletError};

const ELECTRUM_CALL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct ElectrumServerEndpoint {
    pub host: String,
    pub port: u16,
    pub use_tls: bool,
}

impl ElectrumServerEndpoint {
    pub fn parse(uri: &str) -> Result<Self> {
        let trimmed = uri.trim();
        let (use_tls, rest) = if let Some(rest) = trimmed.strip_prefix("tls://") {
            (true, rest)
        } else if let Some(rest) = trimmed.strip_prefix("ssl://") {
            (true, rest)
        } else if let Some(rest) = trimmed.strip_prefix("tcp://") {
            (false, rest)
        } else {
            (true, trimmed)
        };
        let (host, port) = if let Some((h, p)) = rest.rsplit_once(':') {
            let port: u16 = p
                .parse()
                .map_err(|_| WalletError::other(format!("invalid electrum port in {uri}")))?;
            (h.to_string(), port)
        } else {
            (rest.to_string(), if use_tls { 50002 } else { 50001 })
        };
        Ok(Self {
            host,
            port,
            use_tls: use_tls || port != 50001,
        })
    }

    pub fn display(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

enum ElectrumStream {
    Plain(TcpStream),
    Tls(tokio_rustls::client::TlsStream<TcpStream>),
}

impl AsyncRead for ElectrumStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match &mut *self {
            ElectrumStream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            ElectrumStream::Tls(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for ElectrumStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match &mut *self {
            ElectrumStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            ElectrumStream::Tls(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match &mut *self {
            ElectrumStream::Plain(s) => Pin::new(s).poll_flush(cx),
            ElectrumStream::Tls(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match &mut *self {
            ElectrumStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            ElectrumStream::Tls(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

pub struct ElectrumConnection {
    endpoint: ElectrumServerEndpoint,
    io: Mutex<BufReader<ElectrumStream>>,
    next_id: AtomicU64,
    banner: Mutex<Option<String>>,
    connected_at: Instant,
}

impl ElectrumConnection {
    pub async fn connect(endpoint: ElectrumServerEndpoint) -> Result<Arc<Self>> {
        if endpoint.use_tls {
            crate::ensure_tls_crypto_provider();
        }
        let addr = format!("{}:{}", endpoint.host, endpoint.port);
        let tcp = TcpStream::connect(&addr)
            .await
            .map_err(|e| WalletError::other(format!("electrum connect {addr}: {e}")))?;
        tcp.set_nodelay(true).ok();

        let stream = if endpoint.use_tls {
            let mut root_store = rustls::RootCertStore::empty();
            root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let config = rustls::ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth();
            let connector = TlsConnector::from(Arc::new(config));
            let server_name = ServerName::try_from(endpoint.host.as_str())
                .map_err(|_| WalletError::other("invalid electrum TLS server name"))?
                .to_owned();
            let tls = connector
                .connect(server_name, tcp)
                .await
                .map_err(|e| WalletError::other(format!("electrum TLS handshake: {e}")))?;
            ElectrumStream::Tls(tls)
        } else {
            ElectrumStream::Plain(tcp)
        };

        let conn = Arc::new(Self {
            endpoint,
            io: Mutex::new(BufReader::new(stream)),
            next_id: AtomicU64::new(1),
            banner: Mutex::new(None),
            connected_at: Instant::now(),
        });

        let version: Value = conn.call("server.version", json!(["vericonomy-wallet", "1.4"])).await?;
        tracing::debug!("electrum server.version: {version}");
        let banner: String = conn
            .call("server.banner", json!([]))
            .await?
            .as_str()
            .unwrap_or_default()
            .to_string();
        *conn.banner.lock().await = Some(banner);
        Ok(conn)
    }

    pub fn endpoint(&self) -> &ElectrumServerEndpoint {
        &self.endpoint
    }

    pub async fn banner(&self) -> Option<String> {
        self.banner.lock().await.clone()
    }

    pub fn uptime(&self) -> std::time::Duration {
        self.connected_at.elapsed()
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        match timeout(ELECTRUM_CALL_TIMEOUT, self.call_inner(method, params)).await {
            Ok(result) => result,
            Err(_) => Err(WalletError::other(format!(
                "electrum call timed out after {}s: {method}",
                ELECTRUM_CALL_TIMEOUT.as_secs()
            ))),
        }
    }

    async fn call_inner(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = ElectrumRequest {
            id,
            method: method.to_string(),
            params,
        };
        let line = serde_json::to_string(&req)
            .map_err(|e| WalletError::other(format!("electrum encode: {e}")))?
            + "\n";

        let mut io = self.io.lock().await;
        io.get_mut()
            .write_all(line.as_bytes())
            .await
            .map_err(|e| WalletError::other(format!("electrum write: {e}")))?;
        io.get_mut().flush().await.ok();

        loop {
            let mut response_line = String::new();
            let n = io
                .read_line(&mut response_line)
                .await
                .map_err(|e| WalletError::other(format!("electrum read: {e}")))?;
            if n == 0 {
                return Err(WalletError::other("electrum connection closed"));
            }
            let resp: ElectrumResponse = serde_json::from_str(response_line.trim())
                .map_err(|e| WalletError::other(format!("electrum parse: {e}")))?;
            if resp.is_notification() {
                continue;
            }
            if resp.id == Some(id) {
                return resp.into_result();
            }
        }
    }
}
