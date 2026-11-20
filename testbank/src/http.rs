//! Loopback HTTP/1.1 server for the PIN/TAN transport (PIN/TAN B.1:
//! base64 over HTTP POST): one request per connection, `Connection: close`.

use std::ops::Deref;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use crate::engine::{BankHandle, TestBank};
use crate::fault::Fault;

/// Something that answers a decoded FinTS message.
pub(crate) trait Responder: Send + 'static {
    /// `Err` is answered with HTTP 500 and the text.
    fn respond(&mut self, message: &[u8], content_type: Option<String>) -> Result<Vec<u8>, String>;

    /// A fault to deliver instead of the answer just returned.
    fn fault(&mut self) -> Option<Fault> {
        None
    }
}

/// Binds 127.0.0.1 on a free port and serves `responder`.
pub(crate) async fn listen<R: Responder>(responder: R) -> (String, JoinHandle<()>) {
    listen_on(([127, 0, 0, 1], 0).into(), responder).await
}

/// Binds `addr` and serves `responder`.
pub(crate) async fn listen_on<R: Responder>(
    addr: std::net::SocketAddr,
    responder: R,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("bind {addr}: {e}"));
    let url = format!(
        "http://{}/fints",
        listener.local_addr().expect("local address")
    );
    let responder = Arc::new(Mutex::new(responder));
    let task = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let responder = Arc::clone(&responder);
            tokio::spawn(async move {
                let _ = answer(socket, responder).await;
            });
        }
    });
    (url, task)
}

async fn answer<R: Responder>(
    mut socket: TcpStream,
    responder: Arc<Mutex<R>>,
) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let header_end = loop {
        let n = socket.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let header = |name: &str| {
        head.lines().skip(1).find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case(name)
                .then(|| v.trim().to_string())
        })
    };
    let length: usize = header("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    while buf.len() < header_end + length {
        let n = socket.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let body = &buf[header_end..(header_end + length).min(buf.len())];
    let mut fault = None;
    let result = if !head.starts_with("POST ") {
        Err(format!(
            "expected POST, got {}",
            head.lines().next().unwrap_or("")
        ))
    } else {
        match B64.decode(body.trim_ascii()) {
            Ok(message) => {
                let mut responder = responder.lock().unwrap_or_else(|e| e.into_inner());
                let result = responder.respond(&message, header("content-type"));
                if result.is_ok() {
                    fault = responder.fault();
                }
                result
            }
            Err(e) => Err(format!("request body is not base64: {e}")),
        }
    };
    let (status, content) = match result {
        Ok(message) => match fault {
            Some(fault) => match fault.deliver(message).await {
                Some(delivery) => delivery,
                None => return Ok(()),
            },
            None => ("200 OK".to_string(), B64.encode(message)),
        },
        Err(text) => ("500 Internal Server Error".to_string(), text),
    };
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{content}",
        content.len()
    );
    socket.write_all(reply.as_bytes()).await?;
    socket.shutdown().await
}

impl Responder for BankHandle {
    fn fault(&mut self) -> Option<Fault> {
        self.take_fault()
    }

    fn respond(&mut self, message: &[u8], content_type: Option<String>) -> Result<Vec<u8>, String> {
        self.exchange_with(message, content_type)
            .map_err(|v| v.to_string())
    }
}

/// A test bank served over loopback HTTP. Dereferences to its
/// [`BankHandle`]; the server stops when this is dropped.
pub struct HttpBank {
    url: String,
    handle: BankHandle,
    task: JoinHandle<()>,
}

impl TestBank {
    /// Serves the bank on 127.0.0.1 (free port).
    pub async fn serve(self) -> HttpBank {
        let handle = self.handle();
        let (url, task) = listen(handle.clone()).await;
        HttpBank { url, handle, task }
    }
}

impl TestBank {
    /// Serves the bank on `addr` (port 0 = any free port). Use a loopback
    /// address: the bank has no access control.
    pub async fn serve_on(self, addr: std::net::SocketAddr) -> HttpBank {
        let handle = self.handle();
        let (url, task) = listen_on(addr, handle.clone()).await;
        HttpBank { url, handle, task }
    }
}

impl HttpBank {
    /// The FinTS URL, e.g. `http://127.0.0.1:12345/fints`.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The bank's handle (also reachable through `Deref`).
    pub fn handle(&self) -> &BankHandle {
        &self.handle
    }

    /// A `BankConfig` for this bank: name, BLZ and BIC from the model, the
    /// profile the registry has for the model's BLZ.
    pub fn bank_config(&self) -> fints_client::BankConfig {
        let m = self.handle.model();
        let profile = fints_client::banks::profile_by_blz(&m.blz);
        fints_client::BankConfig::new(m.name, m.blz, m.bic, self.url.clone()).with_profile(profile)
    }

    /// A [`BankClient`](fints_client::jobs::BankClient) that talks to this
    /// bank over HTTP.
    pub fn client(&self, product: &fints_client::ProductId) -> fints_client::jobs::BankClient {
        fints_client::jobs::BankClient::new(self.bank_config(), product.clone())
            .with_now(crate::TESTBANK_NOW)
    }
}

impl BankHandle {
    /// A [`BankClient`](fints_client::jobs::BankClient) whose dialogs go
    /// straight to this handle (placeholder URL, no socket).
    pub fn client(&self, product: &fints_client::ProductId) -> fints_client::jobs::BankClient {
        let m = self.model();
        let profile = fints_client::banks::profile_by_blz(&m.blz);
        let config = fints_client::BankConfig::new(m.name, m.blz, m.bic, "in-process://testbank")
            .with_profile(profile);
        fints_client::jobs::BankClient::new(config, product.clone())
            .with_now(crate::TESTBANK_NOW)
            .with_transport(Arc::new(self.clone()))
    }
}

/// In-process transport: the URL is ignored. HTTP-level [`Fault`]s do not
/// apply (there is no HTTP); a [`Violation`](crate::request::Violation) is
/// answered like the server does, as HTTP 500 with the violation text.
impl fints_client::transport::Transport for BankHandle {
    fn send<'a>(
        &'a self,
        _url: &'a str,
        message: &'a [u8],
    ) -> fints_client::transport::BoxFuture<'a, fints_client::Result<Vec<u8>>> {
        Box::pin(async move {
            self.exchange(message)
                .map_err(|v| fints_client::FinTSError::Http {
                    status: 500,
                    message: v.to_string(),
                })
        })
    }
}

impl Deref for HttpBank {
    type Target = BankHandle;
    fn deref(&self) -> &BankHandle {
        &self.handle
    }
}

impl Drop for HttpBank {
    fn drop(&mut self) {
        self.task.abort();
    }
}
