//! Faults the bank delivers instead of a good answer (HTTP level).

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;

/// What goes wrong with one answer. Armed with `TestBank::fault` or
/// `BankHandle::inject`; each fault fires once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// An HTTP error status (e.g. 500, 503) with a short text body.
    Status(u16),
    /// No answer: the connection stays open for this long, then closes
    /// without a reply. Use a delay longer than the client's timeout.
    Silence(Duration),
    /// The decoded answer cut to this many bytes, then base64.
    CutWire(usize),
    /// The base64 text cut to this many characters.
    CutBase64(usize),
    /// Every non-ASCII byte (ISO 8859-1) written as UTF-8 instead, so
    /// "ü" arrives as two bytes. The message size in HNHBK no longer fits.
    Utf8Text,
    /// A body that is not base64.
    InvalidBase64,
}

impl Fault {
    /// Status line and body, or `None` to close without a reply (after
    /// the delay).
    pub(crate) async fn deliver(self, message: Vec<u8>) -> Option<(String, String)> {
        let ok = |body: String| Some(("200 OK".to_string(), body));
        match self {
            Fault::Status(code) => Some((
                format!("{code} {}", reason(code)),
                format!("test bank fault {code}"),
            )),
            Fault::Silence(delay) => {
                tokio::time::sleep(delay).await;
                None
            }
            Fault::CutWire(n) => ok(B64.encode(&message[..n.min(message.len())])),
            Fault::CutBase64(n) => {
                let mut text = B64.encode(message);
                text.truncate(n);
                ok(text)
            }
            Fault::Utf8Text => {
                let text: String = message.iter().map(|&b| b as char).collect();
                ok(B64.encode(text))
            }
            Fault::InvalidBase64 => ok("%%% not base64 %%%".to_string()),
        }
    }
}

fn reason(code: u16) -> &'static str {
    match code {
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Fault",
    }
}
