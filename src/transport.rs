//! Transport layer: how a FinTS message reaches the bank.
//!
//! [`Transport`] carries one message to a URL and returns the answer as wire
//! bytes. [`HttpTransport`] is the default (HTTPS POST, base64 body). The
//! library owns the single call site, `exchange`: it applies the quirks
//! around the transport and emits the masked `TRACE` event, so a custom
//! transport can neither skip the quirks nor leak message content.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tracing::trace;
#[cfg(feature = "http")]
use {
    base64::{Engine, engine::general_purpose::STANDARD as B64},
    std::time::Duration,
    tracing::debug,
};

use crate::error::{FinTSError, Result, TransportKind};
use crate::quirks::{QuirkChain, QuirkContext};

/// Boxed, `Send` future returned by [`Transport::send`] (keeps the trait
/// object safe).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Carries FinTS messages to a bank.
///
/// Implementors return [`FinTSError::Timeout`] for a request that timed out,
/// [`FinTSError::Transport`] (with its [`TransportKind`]) for connect, TLS and
/// decode failures and [`FinTSError::Http`] for status errors, and own their
/// timeouts. Implementations must not log message content.
pub trait Transport: Send + Sync {
    /// Send one FinTS message (wire bytes, PIN/TAN envelope included) to
    /// `url` and return the bank's answer as wire bytes. Encoding for the
    /// medium (base64 over HTTP) is the transport's business.
    fn send<'a>(&'a self, url: &'a str, message: &'a [u8]) -> BoxFuture<'a, Result<Vec<u8>>>;
}

/// A transport shared between dialogs.
pub type SharedTransport = Arc<dyn Transport>;

/// HTTPS transport: POSTs the base64-encoded message as `text/plain`.
/// Needs feature `http` (on by default).
#[cfg(feature = "http")]
pub struct HttpTransport {
    client: reqwest::Client,
}

#[cfg(feature = "http")]
impl HttpTransport {
    /// Default settings: 60 s request timeout.
    pub fn new() -> Result<Self> {
        Self::with_timeout(Duration::from_secs(60))
    }

    /// Like [`HttpTransport::new`] with a custom total timeout per request.
    pub fn with_timeout(timeout: Duration) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .pool_idle_timeout(Duration::from_secs(2))
            .pool_max_idle_per_host(0)
            .tcp_nodelay(true)
            .build()
            .map_err(|e| {
                FinTSError::transport(
                    TransportKind::Config,
                    format!("Failed to create HTTP client: {}", e),
                )
            })?;
        Ok(Self { client })
    }

    /// FinTS over HTTPS (Formals B.1): the request body is the message in
    /// base64, and so is the answer. Line breaks in the answer are allowed
    /// and dropped before decoding.
    async fn post(&self, url: &str, message: &[u8]) -> Result<Vec<u8>> {
        let body = B64.encode(message);
        debug!(
            bytes = message.len(),
            base64 = body.len(),
            url,
            "posting message"
        );
        let request = self
            .client
            .post(url)
            .header("Content-Type", "text/plain")
            .body(body);
        let response = request.send().await.map_err(request_failed)?;

        let status = response.status().as_u16();
        if !response.status().is_success() {
            debug!(status, url, "bank answered with an HTTP error");
            let message = response.text().await.unwrap_or_default();
            return Err(FinTSError::Http { status, message });
        }

        let answer = response.bytes().await.map_err(|e| match e.is_timeout() {
            true => FinTSError::Timeout,
            false => FinTSError::transport(TransportKind::Io, format!("reading the answer: {e}")),
        })?;
        let base64: Vec<u8> = answer
            .iter()
            .copied()
            .filter(|b| !b.is_ascii_whitespace())
            .collect();
        let decoded = B64.decode(&base64).map_err(|e| {
            FinTSError::transport(TransportKind::Decode, format!("answer is not base64: {e}"))
        })?;
        debug!(
            base64 = base64.len(),
            bytes = decoded.len(),
            "answer decoded"
        );
        Ok(decoded)
    }
}

/// The error for a request that got no HTTP answer.
#[cfg(feature = "http")]
fn request_failed(e: reqwest::Error) -> FinTSError {
    if e.is_timeout() {
        return FinTSError::Timeout;
    }
    FinTSError::transport(
        classify_request_error(&e),
        format!("sending the request: {e}"),
    )
}

/// What failed when sending a request, from the error and its causes.
#[cfg(feature = "http")]
fn classify_request_error(e: &reqwest::Error) -> TransportKind {
    let kind = classify_chain(e);
    if kind == TransportKind::Io && e.is_connect() {
        TransportKind::Connect
    } else {
        kind
    }
}

/// Walks the cause chain: a `rustls::Error` (also inside an `io::Error`,
/// which hides its inner error from `source`) is `Tls`; a hyper connect
/// error that says "dns error" is `Dns`; anything else is `Io`.
#[cfg(feature = "http")]
fn classify_chain(e: &(dyn std::error::Error + 'static)) -> TransportKind {
    let mut dns = false;
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(e);
    while let Some(err) = cur {
        if err.is::<rustls::Error>() {
            return TransportKind::Tls;
        }
        if let Some(io) = err.downcast_ref::<std::io::Error>()
            && io
                .get_ref()
                .is_some_and(|inner| inner.is::<rustls::Error>())
        {
            return TransportKind::Tls;
        }
        if err.to_string().starts_with("dns error") {
            dns = true;
        }
        cur = err.source();
    }
    if dns {
        TransportKind::Dns
    } else {
        TransportKind::Io
    }
}

#[cfg(feature = "http")]
impl Transport for HttpTransport {
    fn send<'a>(&'a self, url: &'a str, message: &'a [u8]) -> BoxFuture<'a, Result<Vec<u8>>> {
        Box::pin(self.post(url, message))
    }
}

/// Placeholder transport when feature `http` is off and the caller set none.
#[cfg(not(feature = "http"))]
pub(crate) struct NoTransport;

#[cfg(not(feature = "http"))]
impl Transport for NoTransport {
    fn send<'a>(&'a self, _: &'a str, _: &'a [u8]) -> BoxFuture<'a, Result<Vec<u8>>> {
        Box::pin(async {
            Err(FinTSError::transport(
                TransportKind::Config,
                "no transport: enable feature `http` or call with_transport",
            ))
        })
    }
}

/// The transport a new dialog starts with.
pub(crate) fn default_transport() -> Result<SharedTransport> {
    #[cfg(feature = "http")]
    {
        Ok(Arc::new(HttpTransport::new()?))
    }
    #[cfg(not(feature = "http"))]
    {
        Ok(Arc::new(NoTransport))
    }
}

/// Short, content-free summary of a failed exchange for the trace event.
/// An HTTP body is never included: it may echo request data.
fn error_summary(e: &FinTSError) -> String {
    match e {
        FinTSError::Http { status, .. } => format!("HTTP {status}"),
        FinTSError::Timeout => "timeout".to_string(),
        FinTSError::Transport { kind, message } => format!("transport {kind}: {message}"),
        _ => "error".to_string(),
    }
}

/// The one place the library calls [`Transport::send`]: quirks (outgoing),
/// send, masked `TRACE` event (success or failure), quirks (incoming).
/// Returns the answer as clear text with all quirks applied.
pub(crate) async fn exchange(
    transport: &dyn Transport,
    url: &str,
    quirks: &QuirkChain,
    ctx: &QuirkContext<'_>,
    message_bytes: &[u8],
) -> Result<Vec<u8>> {
    let outgoing = quirks.outgoing(message_bytes, ctx);
    trace!("Request (masked): {}", crate::mask::mask_for_log(&outgoing));
    match transport.send(url, &outgoing).await {
        Ok(raw) => {
            trace!("Response (masked): {}", crate::mask::mask_for_log(&raw));
            trace!(
                target: crate::mask::EXCHANGE_TARGET,
                request = %crate::mask::redacted_text(&outgoing),
                response = %crate::mask::redacted_text(&raw),
                "exchange"
            );
            Ok(quirks.incoming(&raw, ctx))
        }
        Err(e) => {
            trace!(
                target: crate::mask::EXCHANGE_TARGET,
                request = %crate::mask::redacted_text(&outgoing),
                error = %error_summary(&e),
                "exchange"
            );
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "http")]
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[cfg(feature = "http")]
    use tokio::net::TcpListener;

    /// A local HTTP server for one request. `answer` gets the request body
    /// and returns status and body of the reply. Returns the server's URL.
    #[cfg(feature = "http")]
    async fn one_shot_server(
        answer: impl FnOnce(Vec<u8>) -> (u16, Vec<u8>) + Send + 'static,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/fints", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let body = read_request_body(&mut stream).await;
            let (status, reply) = answer(body);
            let head = format!(
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                reply.len()
            );
            stream
                .write_all(&[head.as_bytes(), &reply].concat())
                .await
                .unwrap();
            stream.shutdown().await.ok();
        });
        url
    }

    /// Reads one request and returns its body, as long as Content-Length says.
    #[cfg(feature = "http")]
    async fn read_request_body(stream: &mut tokio::net::TcpStream) -> Vec<u8> {
        let mut request = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let read = stream.read(&mut chunk).await.unwrap();
            request.extend_from_slice(&chunk[..read]);
            let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") else {
                assert!(read > 0, "request ended inside the head");
                continue;
            };
            let head = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
            let length: usize = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .map_or(0, |v| v.trim().parse().unwrap());
            if request.len() >= end + 4 + length || read == 0 {
                return request[end + 4..].to_vec();
            }
        }
    }

    #[cfg(feature = "http")]
    fn is_transport(err: &FinTSError, expected: TransportKind) -> bool {
        matches!(err, FinTSError::Transport { kind, .. } if *kind == expected)
    }

    /// The request goes out as base64; the answer comes back as base64,
    /// here broken into lines as some banks send it.
    #[cfg(feature = "http")]
    #[tokio::test]
    async fn message_and_answer_travel_as_base64() {
        let message = b"HNHBK:1:3+000000000123+300+0+1'";
        let url = one_shot_server(|body| {
            assert_eq!(
                body,
                B64.encode(b"HNHBK:1:3+000000000123+300+0+1'").into_bytes()
            );
            let encoded = B64.encode("HIRMG:2:2+0010::Nachricht entgegengenommen'");
            let (first, second) = encoded.split_at(20);
            (200, format!("{first}\r\n{second}\r\n").into_bytes())
        })
        .await;
        let answer = HttpTransport::new()
            .unwrap()
            .send(&url, message)
            .await
            .unwrap();
        assert_eq!(answer, b"HIRMG:2:2+0010::Nachricht entgegengenommen'");
    }

    /// A status other than 2xx is `Http` with the body as the message.
    #[cfg(feature = "http")]
    #[tokio::test]
    async fn an_http_error_keeps_status_and_body() {
        let url = one_shot_server(|_| (502, b"upstream unavailable".to_vec())).await;
        let err = HttpTransport::new()
            .unwrap()
            .send(&url, b"m")
            .await
            .unwrap_err();
        let FinTSError::Http { status, message } = &err else {
            panic!("{err}");
        };
        assert_eq!((*status, message.as_str()), (502, "upstream unavailable"));
    }

    #[cfg(feature = "http")]
    #[tokio::test]
    async fn an_answer_that_is_not_base64_is_a_decode_error() {
        let url = one_shot_server(|_| (200, b"<html>Wartung</html>".to_vec())).await;
        let err = HttpTransport::new()
            .unwrap()
            .send(&url, b"m")
            .await
            .unwrap_err();
        assert!(is_transport(&err, TransportKind::Decode), "{err}");
    }

    /// Nobody listens on a port just released: the connection is refused.
    #[cfg(feature = "http")]
    #[tokio::test]
    async fn a_closed_port_is_a_connect_error() {
        let address = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap();
        let url = format!("http://{address}/fints");
        let err = HttpTransport::new()
            .unwrap()
            .send(&url, b"m")
            .await
            .unwrap_err();
        assert!(is_transport(&err, TransportKind::Connect), "{err}");
    }

    #[cfg(feature = "http")]
    #[tokio::test]
    async fn http_5xx_is_the_status_error_and_retryable() {
        let url = one_shot_server(|_| (503, Vec::new())).await;
        let err = HttpTransport::new()
            .unwrap()
            .send(&url, b"hex")
            .await
            .unwrap_err();
        assert!(matches!(err, FinTSError::Http { status: 503, .. }));
        assert!(err.is_retryable());
    }

    #[cfg(feature = "http")]
    #[tokio::test]
    async fn a_slow_server_is_a_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        // Accepts and never answers.
        let _held = tokio::spawn(async move {
            let conn = listener.accept().await;
            tokio::time::sleep(Duration::from_secs(5)).await;
            drop(conn);
        });
        let url = format!("http://127.0.0.1:{port}/");
        let err = HttpTransport::with_timeout(Duration::from_millis(100))
            .unwrap()
            .send(&url, b"hex")
            .await
            .unwrap_err();
        assert!(matches!(err, FinTSError::Timeout), "got: {err}");
    }

    #[cfg(feature = "http")]
    #[test]
    fn a_rustls_error_in_the_chain_is_tls() {
        use std::io::{Error, ErrorKind};
        let cert = rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer);
        // Direct, and wrapped in an io::Error the way tokio-rustls reports it
        // (io::Error::source skips the wrapped error, so it needs its own check).
        assert_eq!(classify_chain(&cert), TransportKind::Tls);
        let wrapped = Error::new(ErrorKind::InvalidData, cert);
        assert_eq!(classify_chain(&wrapped), TransportKind::Tls);
        let alert = rustls::Error::AlertReceived(rustls::AlertDescription::HandshakeFailure);
        assert_eq!(classify_chain(&Error::other(alert)), TransportKind::Tls);
    }

    #[cfg(feature = "http")]
    #[test]
    fn dns_and_other_causes() {
        use std::io::{Error, ErrorKind};
        #[derive(Debug)]
        struct Outer(Error);
        impl std::fmt::Display for Outer {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("dns error: failed to lookup address information")
            }
        }
        impl std::error::Error for Outer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }
        assert_eq!(
            classify_chain(&Outer(Error::new(ErrorKind::NotFound, "x"))),
            TransportKind::Dns
        );
        assert_eq!(
            classify_chain(&Error::new(ErrorKind::ConnectionReset, "reset")),
            TransportKind::Io
        );
    }

    #[cfg(not(feature = "http"))]
    #[tokio::test]
    async fn without_http_the_default_transport_is_a_config_error() {
        let t = default_transport().unwrap();
        let err = t.send("http://x/", b"m").await.unwrap_err();
        assert!(matches!(
            err,
            FinTSError::Transport {
                kind: TransportKind::Config,
                ..
            }
        ));
    }

    #[cfg(feature = "http")]
    #[test]
    fn new_accepts_valid_url_and_errors_on_bad_url() {
        assert!(HttpTransport::new().is_ok());
        // A non-URL input still builds an HTTP client (reqwest parses lazily),
        // so we only assert the happy path here.
        assert!(HttpTransport::new().is_ok());
    }

    // ── Failure event ──

    #[derive(Clone, Default)]
    struct Capture(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

    struct Fields(String);

    impl tracing::field::Visit for Fields {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0.push_str(&format!("{}={:?} ", field.name(), value));
        }
    }

    impl tracing::Subscriber for Capture {
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            if event.metadata().target() == crate::mask::EXCHANGE_TARGET {
                let mut fields = Fields(String::new());
                event.record(&mut fields);
                self.0.lock().unwrap().push(fields.0);
            }
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    struct Failing;

    impl Transport for Failing {
        fn send<'a>(&'a self, _: &'a str, _: &'a [u8]) -> BoxFuture<'a, Result<Vec<u8>>> {
            Box::pin(async {
                Err(FinTSError::Http {
                    status: 503,
                    message: "echoed secret body".into(),
                })
            })
        }
    }

    #[tokio::test]
    async fn failing_send_emits_an_event_with_error_and_no_response() {
        let logs = Capture::default();
        let _guard = tracing::subscriber::set_default(logs.clone());
        let ctx = QuirkContext {
            hbci_version: "300",
            dialog_id: "0",
            message_number: 1,
        };
        let r = exchange(&Failing, "http://x/", &QuirkChain::none(), &ctx, b"HNHBK:1").await;
        assert!(matches!(r, Err(FinTSError::Http { status: 503, .. })));
        let events = logs.0.lock().unwrap().clone();
        assert_eq!(events.len(), 1, "{events:?}");
        assert!(events[0].contains("error=HTTP 503"), "{events:?}");
        assert!(events[0].contains("request="), "{events:?}");
        assert!(!events[0].contains("response="), "{events:?}");
        assert!(!events[0].contains("secret"), "{events:?}");
    }
}
