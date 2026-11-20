//! HNHBK, Nachrichtenkopf (message header). Formals 2017, B.5.2.

use super::common::ReferenceMessage;
use crate::codec::{Dig, Id, Num, Segment};

/// HNHBK version 3: the header that starts every message.
///
/// ```text
/// Nr. Name               Typ  Format Länge Status Anzahl
///  1  Segmentkopf        DEG              M     1
///  2  Nachrichtengröße   DE   dig    12   M     1
///  3  HBCI-Version       DE   num   ..3   M     1
///  4  Dialog-ID          DE   id     #    M     1
///  5  Nachrichtennummer  DE   num   ..4   M     1    >0
///  6  Bezugsnachricht    DEG              C     1    M: bank messages, N: customer messages
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HNHBK", version = 3)]
pub struct HnHbk3 {
    /// 2 Nachrichtengröße: total message length in bytes, 12 digits with
    /// leading zeros.
    #[de(len = 12)]
    pub message_size: Dig,
    /// 3 HBCI-Version: `300` for FinTS 3.0.
    #[de(max = 3)]
    pub hbci_version: Num,
    /// 4 Dialog-ID: `0` in the first customer message of a dialog.
    pub dialog_id: Id,
    /// 5 Nachrichtennummer.
    #[de(max = 4)]
    pub message_number: Num,
    /// 6 Bezugsnachricht: the customer message a bank message answers.
    pub reference_message: Option<ReferenceMessage>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HNHBK:1:3+000000000123+300+0+1'",
    "HNHBK:1:3+000000000456+300+DLG?:42+2+DLG?:42:1'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{from_wire, to_wire};
    use proptest::prelude::*;

    fn round_trip(wire: &str) -> HnHbk3 {
        let (header, value) = from_wire::<HnHbk3>(wire.as_bytes()).unwrap();
        let back = to_wire(&value, header.number, header.reference).unwrap();
        assert_eq!(String::from_utf8(back).unwrap(), wire);
        value
    }

    #[test]
    fn customer_header_round_trips() {
        let v = round_trip("HNHBK:1:3+000000000123+300+0+1'");
        assert_eq!(v.message_size.value(), Some(123));
        assert_eq!(v.hbci_version, Num::new(300));
        assert_eq!(v.dialog_id.as_str(), "0");
        assert_eq!(v.reference_message, None);
    }

    #[test]
    fn bank_header_with_reference_message_round_trips() {
        let v = round_trip("HNHBK:1:3+000000000456+300+DLG?:42+2+DLG?:42:1'");
        let r = v.reference_message.unwrap();
        assert_eq!(
            (r.dialog_id.as_str(), r.message_number),
            ("DLG:42", Num::new(1))
        );
    }

    /// The expected wire string for these values.
    #[test]
    fn builds_the_expected_wire() {
        let built = b"HNHBK:1:3+000000000123+300+0+1'".to_vec();
        let derived = HnHbk3 {
            message_size: Dig::zero_padded(123, 12),
            hbci_version: Num::new(300),
            dialog_id: Id::new("0").unwrap(),
            message_number: Num::new(1),
            reference_message: None,
        };
        assert_eq!(to_wire(&derived, 1, None).unwrap(), built);
    }

    #[test]
    fn fixed_and_maximum_lengths_are_enforced() {
        let e = from_wire::<HnHbk3>(b"HNHBK:1:3+123+300+0+1'").unwrap_err();
        assert_eq!((e.de, e.field.as_str()), (Some(2), "message_size"));
        let e = from_wire::<HnHbk3>(b"HNHBK:1:3+000000000123+300+0+12345'").unwrap_err();
        assert_eq!((e.de, e.field.as_str()), (Some(5), "message_number"));
        let e = from_wire::<HnHbk3>(b"HNHBK:1:3+000000000123+300+0+1+DLG'").unwrap_err();
        assert_eq!(e.field, "reference_message.message_number");
    }

    proptest! {
        #[test]
        fn arbitrary_headers_round_trip(
            size in 0u64..1_000_000_000_000,
            dialog in "[A-Za-z0-9+:?@']{1,30}",
            number in 1u64..10_000,
            reference in proptest::option::of((
                "[A-Za-z0-9]{1,30}", 1u64..10_000)),
        ) {
            prop_assume!(dialog.trim() == dialog);
            let v = HnHbk3 {
                message_size: Dig::zero_padded(size, 12),
                hbci_version: Num::new(300),
                dialog_id: Id::new(&dialog).unwrap(),
                message_number: Num::new(number),
                reference_message: reference.map(|(d, n)| ReferenceMessage {
                    dialog_id: Id::new(d).unwrap(),
                    message_number: Num::new(n),
                }),
            };
            let bytes = to_wire(&v, 1, None).unwrap();
            prop_assert_eq!(from_wire::<HnHbk3>(&bytes).unwrap().1, v);
        }
    }
}
