//! HNSHA, Signaturabschluss (signature trailer). HBCI security 2024, B.5.2.
//! For PIN/TAN the "Benutzerdefinierte Signatur" carries PIN and TAN
//! (PIN/TAN 2020, B.9.7 and D).

use super::security::UserDefinedSignature1;
use crate::codec::{An, Bin, Segment};

/// HNSHA version 2.
///
/// ```text
/// Nr. Name                           Typ  Format Länge Status Anzahl
///  1  Segmentkopf                    DEG              M     1
///  2  Sicherheitskontrollreferenz    DE   an     ..14  M     1    <>0
///  3  Validierungsresultat           DE   bin    ..512 C     1    M for HBCI, N for PIN/TAN
///  4  Benutzerdefinierte Signatur    DEG               C     1    N for HBCI; PIN/TAN: PIN[:TAN]
/// ```
///
/// Without a TAN the TAN element is left out (`PIN`, not `PIN:`): the codec
/// cuts trailing empty positions (Formals H.1.5).
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HNSHA", version = 2)]
pub struct HnSha2 {
    /// 2 Sicherheitskontrollreferenz: the value of HNSHK DE 4.
    #[de(max = 14)]
    pub control_reference: An,
    /// 3 Validierungsresultat: the signature for HBCI; PIN/TAN must not
    /// fill it (PIN/TAN B.9.7).
    #[de(max = 512)]
    pub validation_result: Option<Bin>,
    /// 4 Benutzerdefinierte Signatur: PIN and, where required, TAN.
    pub user_signature: Option<UserDefinedSignature1>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HNSHA:7:2+4711++123456:654321'",
    "HNSHA:7:2+4711++123456'",
    "HNSHA:7:2+4711'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::to_wire;

    #[test]
    fn wire_round_trips() {
        let v: HnSha2 = round_trip(SAMPLES[0]);
        let sig = v.user_signature.unwrap();
        assert_eq!(sig.pin.as_str(), "123456");
        assert_eq!(sig.tan.unwrap().as_str(), "654321");
        let v: HnSha2 = round_trip(SAMPLES[1]);
        assert!(v.user_signature.unwrap().tan.is_none());
        let v: HnSha2 = round_trip(SAMPLES[2]);
        assert!(v.user_signature.is_none());
    }

    #[test]
    fn pin_and_tan_stay_out_of_debug_output() {
        let v: HnSha2 = round_trip(SAMPLES[0]);
        let text = format!("{v:?}");
        assert!(
            !text.contains("123456") && !text.contains("654321"),
            "{text}"
        );
    }

    /// The expected wire string for these values.
    #[test]
    fn builds_the_expected_wire() {
        for (tan, built) in [
            (None, &b"HNSHA:7:2+4711++123456'"[..]),
            (Some("654321"), b"HNSHA:7:2+4711++123456:654321'"),
        ] {
            let derived = HnSha2 {
                control_reference: An::new("4711").unwrap(),
                validation_result: None,
                user_signature: Some(UserDefinedSignature1 {
                    pin: An::new("123456").unwrap(),
                    tan: tan.map(|t| An::new(t).unwrap()),
                }),
            };
            assert_eq!(to_wire(&derived, 7, None).unwrap(), built);
        }
    }

    /// Deliberate: without a TAN the TAN element is
    /// left out entirely, `...++PIN'` with no trailing colon (Formals H.1.5,
    /// empty elements at the end of a group are cut). Not `PIN:`.
    #[test]
    fn empty_tan_is_left_out() {
        let ctx = crate::message::SecurityContext {
            dialog_id: "0",
            message_number: 1,
            blz: "12345678",
            user_id: "USER0001",
            system_id: "0",
            pin: "123456",
            tan: Some(""),
            security_function: "921",
        };
        let built = to_wire(&crate::message::hnsha(&ctx, 4711).unwrap(), 7, None).unwrap();
        assert_eq!(built, b"HNSHA:7:2+4711++123456'");
        let (_, v) = crate::codec::from_wire::<HnSha2>(&built).unwrap();
        assert!(v.user_signature.unwrap().tan.is_none());
    }
}
