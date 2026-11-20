//! Repairs for bank messages that break the message layout.
//!
//! A [`QuirkChain`] holds [`Quirk`]s. Each quirk recognises one kind of
//! damage and leaves every other message as it is, so all of them run on
//! every message. An answer from the bank goes through three stages
//! ([`QuirkChain::incoming`]):
//!
//! 1. raw: [`Quirk::incoming_raw`] sees the answer as it arrived, still in
//!    its PIN/TAN envelope (HNVSK, HNVSD). Damage to the message head
//!    HNHBK is repaired here.
//! 2. The envelope is opened: HNVSK goes, HNVSD is replaced by the segments
//!    it carries. What remains is HNHBK, the inner segments and HNHBS.
//! 3. clear text: [`Quirk::incoming`] sees that result.
//!
//! A request passes [`Quirk::outgoing`] before it is sent.
//!
//! [`tolerant`] holds the repairs of the default chain and explains their
//! order.

use std::borrow::Cow;

pub mod tolerant;
pub(crate) mod wire;

/// Facts about the request that an answer belongs to.
#[derive(Debug, Clone, Copy)]
pub struct QuirkContext<'a> {
    /// The HBCI version in our HNHBK, normally `300`.
    pub hbci_version: &'a str,
    /// Our dialog ID; `0` in the first message of a dialog.
    pub dialog_id: &'a str,
    /// Our message number.
    pub message_number: u16,
}

/// The repair of one kind of damage.
///
/// Each method returns its input unchanged unless overridden. A method must
/// not panic, and it returns `Cow::Borrowed` for a message it does not
/// change; the chain logs every `Cow::Owned` as a repair.
pub trait Quirk: Send + Sync {
    /// The name in log lines, such as `SegmentNumbering`.
    fn name(&self) -> &'static str;

    /// Repair an answer as it arrived, envelope included.
    fn incoming_raw<'a>(&self, msg: &'a [u8], _ctx: &QuirkContext<'_>) -> Cow<'a, [u8]> {
        Cow::Borrowed(msg)
    }

    /// Repair an answer after the envelope was opened.
    fn incoming<'a>(&self, msg: &'a [u8], _ctx: &QuirkContext<'_>) -> Cow<'a, [u8]> {
        Cow::Borrowed(msg)
    }

    /// Change a request before it is sent.
    fn outgoing<'a>(&self, msg: &'a [u8], _ctx: &QuirkContext<'_>) -> Cow<'a, [u8]> {
        Cow::Borrowed(msg)
    }
}

/// The quirks of one connection, in the order they run.
pub struct QuirkChain {
    quirks: Vec<Box<dyn Quirk>>,
}

/// The three places a quirk can act.
#[derive(Debug, Clone, Copy)]
enum Stage {
    Raw,
    ClearText,
    Outgoing,
}

impl Stage {
    fn apply<'a>(self, quirk: &dyn Quirk, msg: &'a [u8], ctx: &QuirkContext<'_>) -> Cow<'a, [u8]> {
        match self {
            Self::Raw => quirk.incoming_raw(msg, ctx),
            Self::ClearText => quirk.incoming(msg, ctx),
            Self::Outgoing => quirk.outgoing(msg, ctx),
        }
    }
}

impl std::fmt::Debug for QuirkChain {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.debug_tuple("QuirkChain").field(&self.names()).finish()
    }
}

impl Default for QuirkChain {
    /// The repairs of [`tolerant`]: [`MissingMessageReference`](tolerant::MissingMessageReference),
    /// [`HeaderVersion`](tolerant::HeaderVersion),
    /// [`StatusSegmentOrder`](tolerant::StatusSegmentOrder) and
    /// [`SegmentNumbering`](tolerant::SegmentNumbering), in that order.
    fn default() -> Self {
        let mut chain = Self::none();
        chain.quirks = vec![
            Box::new(tolerant::MissingMessageReference),
            Box::new(tolerant::HeaderVersion),
            Box::new(tolerant::StatusSegmentOrder),
            Box::new(tolerant::SegmentNumbering),
        ];
        chain
    }
}

impl QuirkChain {
    /// An empty chain. The envelope of answers is still opened.
    pub fn none() -> Self {
        Self { quirks: Vec::new() }
    }

    /// The chain with `quirk` added at the end, for example a repair for
    /// one bank.
    pub fn with(mut self, quirk: impl Quirk + 'static) -> Self {
        self.quirks.push(Box::new(quirk));
        self
    }

    /// The names of the quirks, in order.
    pub fn names(&self) -> Vec<&'static str> {
        self.quirks.iter().map(|quirk| quirk.name()).collect()
    }

    /// An answer from the bank, repaired and taken out of its envelope.
    pub fn incoming(&self, raw: &[u8], ctx: &QuirkContext<'_>) -> Vec<u8> {
        let repaired = self.run(Stage::Raw, raw.to_vec(), ctx);
        self.run(Stage::ClearText, unwrap_envelope(&repaired), ctx)
    }

    /// A request, changed by the outgoing quirks.
    pub fn outgoing(&self, msg: &[u8], ctx: &QuirkContext<'_>) -> Vec<u8> {
        self.run(Stage::Outgoing, msg.to_vec(), ctx)
    }

    fn run(&self, stage: Stage, mut msg: Vec<u8>, ctx: &QuirkContext<'_>) -> Vec<u8> {
        for quirk in &self.quirks {
            if let Cow::Owned(repaired) = stage.apply(quirk.as_ref(), &msg, ctx) {
                tracing::warn!(
                    quirk = quirk.name(),
                    ?stage,
                    "[FinTS] bank message repaired"
                );
                msg = repaired;
            }
        }
        msg
    }
}

/// The clear-text form of an answer: HNVSK dropped and HNVSD replaced by
/// the segments in its binary element. An answer without envelope (some
/// banks send errors that way) comes back unchanged.
pub(crate) fn unwrap_envelope(msg: &[u8]) -> Vec<u8> {
    let mut clear = Vec::with_capacity(msg.len());
    for segment in wire::segments(msg).into_iter().map(|range| &msg[range]) {
        let keep = match wire::segment_type(segment) {
            b"HNVSK" => continue,
            b"HNVSD" => encrypted_data(segment).unwrap_or(segment),
            _ => segment,
        };
        clear.extend_from_slice(keep);
    }
    clear
}

/// The binary content of HNVSD's data element "Verschlüsselte Daten".
fn encrypted_data(hnvsd: &[u8]) -> Option<&[u8]> {
    let group = wire::degs(hnvsd).into_iter().nth(1)?;
    wire::binary_payload(&hnvsd[group])
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTX: QuirkContext<'static> = QuirkContext {
        hbci_version: "300",
        dialog_id: "D-1",
        message_number: 3,
    };

    const HEAD: &str = "HNHBK:1:3+000000000120+300+D-1+3+D-1:3'";

    fn enveloped(inner: &str) -> Vec<u8> {
        format!(
            "{HEAD}HNVSK:998:3+PIN:2+998+1+2::sys+1:20260101:120000+2:2:13:@8@00000000:5:1+280:10000000:user:V:0:0+0'\
             HNVSD:999:1+@{}@{inner}'HNHBS:5:1+3'",
            inner.len()
        )
        .into_bytes()
    }

    #[test]
    fn envelope_is_replaced_by_its_content() {
        let inner =
            "HIRMG:2:2+0010::Nachricht entgegengenommen.'HIRMS:3:2:3+0020::Auftrag ausgeführt.'";
        let clear = format!("{HEAD}{inner}HNHBS:5:1+3'");
        assert_eq!(unwrap_envelope(&enveloped(inner)), clear.as_bytes());
    }

    #[test]
    fn answer_without_envelope_is_unchanged() {
        let plain = format!("{HEAD}HIRMG:2:2+9800::Dialog abgebrochen.'HNHBS:3:1+3'");
        assert_eq!(unwrap_envelope(plain.as_bytes()), plain.as_bytes());
    }

    #[test]
    fn default_chain_leaves_a_sound_answer_alone() {
        let inner = "HIRMG:2:2+0010::ok'HIRMS:3:2:3+0020::ok'HISAL:4:7:3+x'";
        let clear = format!("{HEAD}{inner}HNHBS:5:1+3'");
        let chain = QuirkChain::default();
        assert_eq!(chain.incoming(&enveloped(inner), &CTX), clear.as_bytes());
        assert_eq!(chain.outgoing(clear.as_bytes(), &CTX), clear.as_bytes());
    }

    /// Test quirk: replaces every `x` by `y` in the stage it is built for.
    struct XToY(Stage);

    impl XToY {
        fn swap<'a>(&self, stage: Stage, msg: &'a [u8]) -> Cow<'a, [u8]> {
            let wanted = std::mem::discriminant(&self.0) == std::mem::discriminant(&stage);
            if wanted && msg.contains(&b'x') {
                Cow::Owned(
                    msg.iter()
                        .map(|&b| if b == b'x' { b'y' } else { b })
                        .collect(),
                )
            } else {
                Cow::Borrowed(msg)
            }
        }
    }

    impl Quirk for XToY {
        fn name(&self) -> &'static str {
            "XToY"
        }
        fn incoming_raw<'a>(&self, msg: &'a [u8], _: &QuirkContext<'_>) -> Cow<'a, [u8]> {
            self.swap(Stage::Raw, msg)
        }
        fn incoming<'a>(&self, msg: &'a [u8], _: &QuirkContext<'_>) -> Cow<'a, [u8]> {
            self.swap(Stage::ClearText, msg)
        }
        fn outgoing<'a>(&self, msg: &'a [u8], _: &QuirkContext<'_>) -> Cow<'a, [u8]> {
            self.swap(Stage::Outgoing, msg)
        }
    }

    #[test]
    fn each_stage_sees_its_own_form_of_the_message() {
        let answer = enveloped("HIRMG:2:2+0010::x'");
        // Raw stage: the envelope is still there; the repair reaches the
        // inner segments too, as their length does not change.
        let raw = QuirkChain::none()
            .with(XToY(Stage::Raw))
            .incoming(&answer, &CTX);
        assert!(raw.windows(9).any(|w| w == b"0010::y'H"));
        // Clear text: no HNVSK is left to see.
        let clear = QuirkChain::none()
            .with(XToY(Stage::ClearText))
            .incoming(&answer, &CTX);
        assert!(!clear.windows(5).any(|w| w == b"HNVSK"));
        assert!(clear.windows(8).any(|w| w == b"0010::y'"));
        // Outgoing quirks do not touch answers, and the other way round.
        let out_only = QuirkChain::none().with(XToY(Stage::Outgoing));
        assert!(out_only.incoming(&answer, &CTX).contains(&b'x'));
        assert_eq!(out_only.outgoing(b"x", &CTX), b"y");
    }

    #[test]
    fn names_in_order() {
        assert!(QuirkChain::none().names().is_empty());
        let chain = QuirkChain::default().with(XToY(Stage::Raw));
        let names = chain.names();
        assert_eq!(names.len(), 5);
        assert_eq!(names.first(), Some(&"MissingMessageReference"));
        assert_eq!(names.last(), Some(&"XToY"));
    }
}
