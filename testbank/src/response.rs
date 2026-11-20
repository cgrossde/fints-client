//! Bank answers: feedback (HIRMG/HIRMS), data segments, and the envelope
//! (HNHBK, HNVSK, HNVSD, HNHBS) around them.
//!
//! Segments are written with the codec's definitions
//! (`fints_client::codec::defs`); the bank assigns numbers and references.
//! Layout of a bank message (PIN/TAN F.2): `HNHBK:1`, `HNVSK:998`,
//! `HNVSD:999` holding `HIRMG:2`, `HIRMS:3…` and the data segments, then
//! `HNHBS` numbered after the last inner segment. A bank message has no
//! signature (HNSHK/HNSHA).

use fints_client::codec::defs::common::{Kik1, ReferenceMessage};
use fints_client::codec::defs::security::{
    EncryptionAlgorithm2, KeyName3, SecurityIdentification2, SecurityProfile1, SecurityTimestamp2,
};
use fints_client::codec::defs::{Feedback1, HiRmg2, HiRms2, HnHbk3, HnHbs1, HnVsd1, HnVsk3};
use fints_client::codec::{
    self, An, Bin, Code, Dat, Dig, Id, Num, RawSegment, SegmentHeader, Tim, TypedSegment, wire,
};
use jiff::civil::Date;

/// One return code ("Rückmeldung").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feedback {
    /// Four-digit code, e.g. `0020`.
    pub code: String,
    /// Text, at most 80 characters.
    pub text: String,
    /// Parameters (e.g. the touchdown point of 3040).
    pub params: Vec<String>,
}

impl Feedback {
    /// A code with a text and no parameters.
    pub fn new(code: &str, text: &str) -> Self {
        Self {
            code: code.into(),
            text: text.into(),
            params: Vec::new(),
        }
    }

    /// Adds parameters.
    pub fn with_params(mut self, params: &[&str]) -> Self {
        self.params = params.iter().map(|p| p.to_string()).collect();
        self
    }

    fn is_error(&self) -> bool {
        self.code.starts_with('9')
    }

    fn is_warning(&self) -> bool {
        self.code.starts_with('3')
    }
}

/// A data segment of an answer.
#[derive(Debug, Clone)]
pub enum Reply {
    /// Written with its codec definition.
    Typed(Box<TypedSegment>),
    /// Written as given: `code:n:version:ref+body` (body in wire syntax).
    Raw {
        code: &'static str,
        version: u16,
        body: String,
    },
}

impl<T: Into<TypedSegment>> From<T> for Reply {
    fn from(v: T) -> Self {
        Reply::Typed(Box::new(v.into()))
    }
}

/// The bank's answer to one customer segment.
#[derive(Debug, Clone, Default)]
pub struct Answer {
    /// Codes for the whole message (HIRMG). Empty: derived from the
    /// segment codes of all answers (9050 if any error, 3060 if any
    /// warning, else 0010).
    pub message: Vec<Feedback>,
    /// Codes for the segment (HIRMS); no HIRMS when empty.
    pub segment: Vec<Feedback>,
    /// Data segments referring to the segment.
    pub data: Vec<Reply>,
}

impl Answer {
    /// 0020 with data.
    pub fn ok(data: Vec<Reply>) -> Self {
        Self {
            message: Vec::new(),
            segment: vec![Feedback::new("0020", "Auftrag ausgeführt")],
            data,
        }
    }

    /// One segment code, no data.
    pub fn code(code: &str, text: &str) -> Self {
        Self {
            segment: vec![Feedback::new(code, text)],
            ..Self::default()
        }
    }

    /// An error code on the segment (HIRMG becomes 9050).
    pub fn error(code: &str, text: &str) -> Self {
        Self::code(code, text)
    }

    /// Adds a segment code.
    pub fn with(mut self, feedback: Feedback) -> Self {
        self.segment.push(feedback);
        self
    }

    /// Adds a data segment.
    pub fn with_data(mut self, reply: impl Into<Reply>) -> Self {
        self.data.push(reply.into());
        self
    }
}

/// Text for HIRMG 0010.
pub const MESSAGE_OK: &str = "Nachricht entgegengenommen";
/// Text for HIRMG 3060.
pub const MESSAGE_WARNINGS: &str = "Bitte beachten Sie die enthaltenen Warnungen/Hinweise";
/// Text for HIRMG 9050.
pub const MESSAGE_ERRORS: &str = "Die Nachricht enthält Fehler";

fn an(s: &str) -> An {
    An::new(s).unwrap_or_else(|e| panic!("text {s:?}: {e}"))
}

fn feedback1(f: &Feedback) -> Feedback1 {
    Feedback1 {
        code: Dig::new(&f.code).unwrap_or_else(|e| panic!("code {:?}: {e}", f.code)),
        reference_element: None,
        text: an(&f.text),
        parameters: f.params.iter().map(|p| an(p)).collect(),
    }
}

fn encode(segment: &TypedSegment, number: u16, reference: Option<u16>) -> Vec<u8> {
    let header = SegmentHeader {
        code: segment.code().into(),
        number,
        version: segment.version(),
        reference,
    };
    let mut degs = vec![header.encode()];
    degs.extend(
        segment
            .encode_body()
            .unwrap_or_else(|e| panic!("bank segment {}: {e}", segment.code())),
    );
    wire::serialize_segment(&RawSegment { degs })
        .unwrap_or_else(|e| panic!("bank segment {}: {e}", segment.code()))
}

/// ISO 8859-1 bytes of a text (characters above U+00FF become `?`).
pub(crate) fn to_latin1(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
        .collect()
}

/// The inner segments of a bank message (HIRMG, HIRMS and data), each one
/// complete with its terminator, numbered from 2.
pub(crate) fn inner_segments(answers: &[(u16, Answer)]) -> Vec<Vec<u8>> {
    let mut message: Vec<Feedback> = answers
        .iter()
        .flat_map(|(_, a)| a.message.iter().cloned())
        .collect();
    if message.is_empty() {
        let all = || answers.iter().flat_map(|(_, a)| a.segment.iter());
        message.push(if all().any(Feedback::is_error) {
            Feedback::new("9050", MESSAGE_ERRORS)
        } else if all().any(Feedback::is_warning) {
            Feedback::new("3060", MESSAGE_WARNINGS)
        } else {
            Feedback::new("0010", MESSAGE_OK)
        });
    }
    let mut out = Vec::new();
    let mut number = 2u16;
    let hirmg = HiRmg2 {
        feedback: message.iter().map(feedback1).collect(),
    };
    out.push(encode(&hirmg.into(), number, None));
    for (reference, answer) in answers {
        if !answer.segment.is_empty() {
            number += 1;
            let hirms = HiRms2 {
                feedback: answer.segment.iter().map(feedback1).collect(),
            };
            out.push(encode(&hirms.into(), number, Some(*reference)));
        }
    }
    for (reference, answer) in answers {
        for reply in &answer.data {
            number += 1;
            out.push(match reply {
                Reply::Typed(t) => encode(t, number, Some(*reference)),
                Reply::Raw {
                    code,
                    version,
                    body,
                } => to_latin1(&format!("{code}:{number}:{version}:{reference}+{body}'")),
            });
        }
    }
    out
}

/// Who the envelope names.
pub(crate) struct Envelope<'a> {
    pub dialog_id: &'a str,
    pub message_number: u32,
    pub blz: &'a str,
    pub user_id: &'a str,
    pub system_id: &'a str,
    /// Security function of the request (decides PIN:1 or PIN:2).
    pub security_function: &'a str,
}

/// Fixed timestamp of the bank's HNVSK, so answers are reproducible.
fn stamp() -> (Date, jiff::civil::Time) {
    (
        jiff::civil::date(2026, 1, 1),
        jiff::civil::time(12, 0, 0, 0),
    )
}

fn code(s: &str) -> Code {
    Code::new(s).unwrap_or_else(|e| panic!("code {s:?}: {e}"))
}

fn id(s: &str) -> Id {
    Id::new(s).unwrap_or_else(|e| panic!("id {s:?}: {e}"))
}

/// Wraps inner segments (already numbered from 2) into a bank message.
pub(crate) fn envelope(env: &Envelope<'_>, inner: &[Vec<u8>]) -> Vec<u8> {
    let (date, time) = stamp();
    let profile_version = match env.security_function.parse::<u16>() {
        Ok(f) if f >= 998 => 1,
        _ => 2,
    };
    let hnvsk = HnVsk3 {
        profile: SecurityProfile1 {
            method: code("PIN"),
            version: Num::new(profile_version),
        },
        security_function: code("998"),
        role: code("1"),
        identification: SecurityIdentification2 {
            party: code("1"),
            cid: None,
            party_id: Some(id(env.system_id)),
        },
        timestamp: SecurityTimestamp2 {
            kind: code("1"),
            date: Some(Dat::new(date).expect("date")),
            time: Some(Tim::new(time).expect("time")),
        },
        encryption_algorithm: EncryptionAlgorithm2 {
            usage: code("2"),
            mode: code("2"),
            algorithm: code("13"),
            key_value: Bin(vec![0; 8]),
            key_parameter_name: code("5"),
            iv_parameter_name: code("1"),
            iv_value: None,
        },
        key_name: KeyName3 {
            bank: Kik1 {
                country: Dig::new("280").expect("country"),
                bank_code: Some(an(env.blz)),
            },
            user_id: id(env.user_id),
            key_type: code("V"),
            key_number: Num::new(0),
            key_version: Num::new(0),
        },
        compression: code("0"),
        certificate: None,
    };
    let clear: Vec<u8> = inner.concat();
    let last_inner = 1 + inner.len() as u16;
    let rest = [
        codec::to_wire(&hnvsk, 998, None).expect("HNVSK"),
        codec::to_wire(&HnVsd1 { data: Bin(clear) }, 999, None).expect("HNVSD"),
        codec::to_wire(
            &HnHbs1 {
                message_number: Num::new(u64::from(env.message_number)),
            },
            last_inner + 1,
            None,
        )
        .expect("HNHBS"),
    ]
    .concat();
    let head = |size: usize| {
        codec::to_wire(
            &HnHbk3 {
                message_size: Dig::zero_padded(size as u64, 12),
                hbci_version: Num::new(300),
                dialog_id: id(env.dialog_id),
                message_number: Num::new(u64::from(env.message_number)),
                reference_message: Some(ReferenceMessage {
                    dialog_id: id(env.dialog_id),
                    message_number: Num::new(u64::from(env.message_number)),
                }),
            },
            1,
            None,
        )
        .expect("HNHBK")
    };
    // The size field has a fixed width: one pass to measure, one to write.
    let size = head(0).len() + rest.len();
    [head(size), rest].concat()
}
