//! HISPA, SEPA-Kontoverbindung rückmelden. Messages 2022, C.10.1.3
//! (response version 1, used with HKSPA 1–3).

use super::common::Ktz1;
use crate::codec::Segment;

/// HISPA version 1, bank response.
///
/// ```text
/// Nr. Name                  Typ  Format Länge Status Anzahl
///  1  Segmentkopf           DEG               M     1
///  2  SEPA-Kontoverbindung  DEG  ktz     #    O     ..999
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HISPA", version = 1)]
pub struct HiSpa1 {
    /// 2 SEPA-Kontoverbindung, zero or more.
    #[de(max_count = 999)]
    pub accounts: Vec<Ktz1>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HISPA:4:1:3'",
    "HISPA:4:1:3+J:DE89370400440532013000:COBADEFFXXX:1234567890::280:12345678'",
    "HISPA:4:1:3+J:DE89370400440532013000:COBADEFFXXX:1234567890::280:12345678+N:::9876543210::280:12345678'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Jn;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HiSpa1 = round_trip(SAMPLES[0]);
        assert!(v.accounts.is_empty());
        let v: HiSpa1 = round_trip(SAMPLES[1]);
        assert_eq!(v.accounts.len(), 1);
        assert_eq!(v.accounts[0].sepa_usable, Jn::YES);
        assert_eq!(
            v.accounts[0].iban.as_ref().unwrap().as_str(),
            "DE89370400440532013000"
        );
        let v: HiSpa1 = round_trip(SAMPLES[2]);
        assert_eq!(v.accounts[1].sepa_usable, Jn::NO);
        assert!(v.accounts[1].iban.is_none());
        assert_eq!(v.accounts[1].account_number.as_str(), "9876543210");
    }

    #[test]
    fn rules_are_enforced() {
        let e = from_wire::<HiSpa1>(
            b"HISPA:4:1:3+X:DE89370400440532013000:COBADEFFXXX:1234567890::280:12345678'",
        )
        .unwrap_err();
        assert_eq!(e.field, "accounts[0].sepa_usable");
        let e =
            from_wire::<HiSpa1>(b"HISPA:4:1:3+J:DE89370400440532013000:COBADEFFXXX::280:12345678'")
                .unwrap_err();
        assert_eq!(e.field, "accounts[0].account_number");
    }
}
