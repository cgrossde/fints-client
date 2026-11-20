//! A customer message as the bank received it, and the strict checks it
//! must pass.
//!
//! One path: tokenize with `codec::wire`, read every segment with its
//! definition. Anything the codec cannot read, and every break of the
//! message rules below, is a [`Violation`]: a bug in the client, not bank
//! behaviour.

use fints_client::codec::defs::{HnHbk3, HnHbs1, HnSha2, HnShk4, HnVsd1, HnVsk3};
use fints_client::codec::{self, ParsedSegment, RawSegment, Segment, TypedSegment, wire};

/// A message rule the client broke. Answered with HTTP 500 and recorded.
/// Displayed as `<exchange> <message>`, the line format of a replay's
/// `.expect` sidecar (`replay::expected_violations`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// Index of the exchange (0-based, counting every request).
    pub exchange: usize,
    /// What was wrong.
    pub message: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.exchange, self.message)
    }
}

/// One customer message.
#[derive(Debug, Clone)]
pub struct Request {
    /// Index of the exchange (0-based).
    pub index: usize,
    /// Dialog ID from HNHBK (`0` for a new dialog).
    pub dialog_id: String,
    /// Message number from HNHBK.
    pub message_number: u32,
    /// The decoded message (after base64).
    pub raw: Vec<u8>,
    /// The segments inside HNVSD (HNSHK … HNSHA), read with the codec.
    pub segments: Vec<ParsedSegment>,
    /// Security function from HNSHK.
    pub security_function: Option<String>,
    /// PIN from HNSHA.
    pub pin: Option<String>,
    /// TAN from HNSHA.
    pub tan: Option<String>,
    /// The HTTP `Content-Type` header, when the request came over HTTP.
    pub http_content_type: Option<String>,
}

impl Request {
    /// Segment codes inside HNVSD, in order.
    pub fn codes(&self) -> Vec<&str> {
        self.segments.iter().map(ParsedSegment::code).collect()
    }

    /// Whether a segment with this code is inside HNVSD.
    pub fn has(&self, code: &str) -> bool {
        self.segments.iter().any(|s| s.code() == code)
    }

    /// The first inner segment that reads as `T`.
    pub fn get<T: Segment>(&self) -> Option<T> {
        self.all::<T>().into_iter().next()
    }

    /// Every inner segment that reads as `T`.
    pub fn all<T: Segment>(&self) -> Vec<T> {
        self.segments
            .iter()
            .filter(|s| s.code() == T::CODE)
            .filter_map(|s| s.to_raw().ok())
            .filter_map(|raw| codec::decode_segment::<T>(&raw).ok().map(|(_, v)| v))
            .collect()
    }

    /// The typed inner segments.
    pub fn typed(&self) -> impl Iterator<Item = &TypedSegment> {
        self.segments.iter().filter_map(ParsedSegment::typed)
    }

    /// The whole message as text (ISO 8859-1), for string assertions.
    pub fn text(&self) -> String {
        latin1(&self.raw)
    }

    /// The inner segment `code` with its number.
    pub(crate) fn find(&self, code: &str) -> Option<(u16, RawSegment)> {
        self.segments.iter().find_map(|s| match s {
            ParsedSegment::Typed { header, .. } if header.code == code => {
                Some((header.number, s.to_raw().ok()?))
            }
            _ => None,
        })
    }
}

/// Bytes as ISO 8859-1 text.
pub fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

fn typed<T: Segment>(raw: &RawSegment, what: &str) -> Result<(u16, T), String> {
    codec::decode_segment::<T>(raw)
        .map(|(h, v)| (h.number, v))
        .map_err(|e| format!("{what} unreadable: {e}"))
}

/// Reads and checks a customer message.
pub(crate) fn parse(
    index: usize,
    bytes: &[u8],
    http_content_type: Option<String>,
) -> Result<Request, String> {
    let outer = wire::tokenize(bytes).map_err(|e| format!("message does not tokenize: {e}"))?;
    let codes: Vec<&str> = outer.iter().map(RawSegment::segment_type).collect();
    if codes != ["HNHBK", "HNVSK", "HNVSD", "HNHBS"] {
        return Err(format!(
            "envelope must be HNHBK, HNVSK, HNVSD, HNHBS; got {codes:?}"
        ));
    }
    let (hbk_number, hnhbk) = typed::<HnHbk3>(&outer[0], "HNHBK")?;
    let (vsk_number, _) = typed::<HnVsk3>(&outer[1], "HNVSK")?;
    let (vsd_number, hnvsd) = typed::<HnVsd1>(&outer[2], "HNVSD")?;
    let (hbs_number, hnhbs) = typed::<HnHbs1>(&outer[3], "HNHBS")?;
    if (hbk_number, vsk_number, vsd_number) != (1, 998, 999) {
        return Err(format!(
            "envelope numbers must be 1, 998, 999; got {hbk_number}, {vsk_number}, {vsd_number}"
        ));
    }
    if hnhbk.message_size.value() != Some(bytes.len() as u64) {
        return Err(format!(
            "HNHBK size {} but the message has {} bytes",
            hnhbk.message_size,
            bytes.len()
        ));
    }
    if hnhbk.reference_message.is_some() {
        return Err("a customer HNHBK carries no reference message".into());
    }
    let message_number = hnhbk.message_number.value();
    if hnhbs.message_number.value() != message_number {
        return Err(format!(
            "HNHBS message number {} differs from HNHBK {message_number}",
            hnhbs.message_number
        ));
    }

    let inner = wire::tokenize(&hnvsd.data.0)
        .map_err(|e| format!("HNVSD content does not tokenize: {e}"))?;
    let segments = codec::parse_segments(inner);
    for s in &segments {
        if let ParsedSegment::Raw { error, segment } = s {
            return Err(format!(
                "segment {} unreadable: {error}",
                segment.segment_type()
            ));
        }
    }
    let numbers: Vec<u16> = segments
        .iter()
        .filter_map(|s| match s {
            ParsedSegment::Typed { header, .. } => Some(header.number),
            ParsedSegment::Raw { .. } => None,
        })
        .collect();
    let expected: Vec<u16> = (2..).take(segments.len()).collect();
    if numbers != expected {
        return Err(format!(
            "inner segments must be numbered from 2; got {numbers:?}"
        ));
    }
    if usize::from(hbs_number) != segments.len() + 2 {
        return Err(format!(
            "HNHBS must be number {}; got {hbs_number}",
            segments.len() + 2
        ));
    }
    if segments.first().map(ParsedSegment::code) != Some("HNSHK")
        || segments.last().map(ParsedSegment::code) != Some("HNSHA")
    {
        return Err("HNVSD must start with HNSHK and end with HNSHA".into());
    }

    let mut request = Request {
        index,
        dialog_id: hnhbk.dialog_id.as_str().to_string(),
        message_number: u32::try_from(message_number).map_err(|_| "message number too large")?,
        raw: bytes.to_vec(),
        segments,
        security_function: None,
        pin: None,
        tan: None,
        http_content_type,
    };
    let shk: HnShk4 = request.get().ok_or("HNSHK unreadable")?;
    let sha: HnSha2 = request.get().ok_or("HNSHA unreadable")?;
    if shk.control_reference != sha.control_reference {
        return Err("HNSHK and HNSHA control references differ".into());
    }
    request.security_function = Some(shk.security_function.as_str().to_string());
    if let Some(sig) = sha.user_signature {
        request.pin = Some(sig.pin.as_str().to_string());
        request.tan = sig.tan.map(|t| t.as_str().to_string());
    }
    Ok(request)
}
