//! HKSPA, SEPA-Kontoverbindung anfordern. Messages 2022, C.10.1.3
//! (segment version 1).

use super::common::Ktv3;
use crate::codec::Segment;

/// HKSPA version 1, customer order.
///
/// ```text
/// Nr. Name              Typ  Format Länge Status Anzahl
///  1  Segmentkopf       DEG               M     1
///  2  Kontoverbindung   DEG  ktv     #    O     ..999
/// ```
///
/// An empty body asks for every SEPA account; each `Ktv3` restricts the
/// answer to that account.
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKSPA", version = 1)]
pub struct HkSpa1 {
    /// 2 Kontoverbindung, zero or more.
    #[de(max_count = 999)]
    pub accounts: Vec<Ktv3>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HKSPA:3:1'",
    "HKSPA:3:1+1234567890::280:12345678'",
    "HKSPA:3:1+1234567890::280:12345678+9876543210:01:280:12345678'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::{from_wire, to_wire};

    #[test]
    fn wire_round_trips() {
        let v: HkSpa1 = round_trip(SAMPLES[0]);
        assert!(v.accounts.is_empty());
        let v: HkSpa1 = round_trip(SAMPLES[1]);
        assert_eq!(v.accounts[0].account_number.as_str(), "1234567890");
        assert!(v.accounts[0].subaccount.is_none());
        let v: HkSpa1 = round_trip(SAMPLES[2]);
        assert_eq!(v.accounts.len(), 2);
        assert_eq!(v.accounts[1].subaccount.as_ref().unwrap().as_str(), "01");
    }

    #[test]
    fn builds_the_expected_wire() {
        // The protocol sends no HKSPA; the codec still builds it.
        let built = b"HKSPA:3:1'".to_vec();
        assert_eq!(
            to_wire(&HkSpa1 { accounts: vec![] }, 3, None).unwrap(),
            built
        );
    }

    #[test]
    fn rules_are_enforced() {
        let e = from_wire::<HkSpa1>(b"HKSPA:3:1+1234567890::28:12345678'").unwrap_err();
        assert_eq!(e.field, "accounts[0].bank.country");
        let e = from_wire::<HkSpa1>(b"HKSPA:3:2'").unwrap_err();
        assert!(matches!(
            e.kind,
            crate::codec::ErrorKind::WrongSegment { .. }
        ));
    }
}
