//! HKSYN, Synchronisierung (synchronisation). Formals 2017, C.8.1.2.

use crate::codec::{Code, Segment};

/// HKSYN version 3: asks the bank for a new customer system ID, the last
/// processed message number or the signature ID.
///
/// ```text
/// Nr. Name                Typ  Format Länge Status Anzahl
///  1  Segmentkopf         DEG              M     1
///  2  Synchronisierungsmodus DE code  1    M     1    0, 1, 2
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKSYN", version = 3)]
pub struct HkSyn3 {
    /// 2 Synchronisierungsmodus: `0` new customer system ID, `1` last
    /// processed message number, `2` signature ID.
    #[de(len = 1)]
    pub mode: Code,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &["HKSYN:5:3+0'"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::{from_wire, to_wire};

    #[test]
    fn wire_round_trips() {
        let v: HkSyn3 = round_trip(SAMPLES[0]);
        assert_eq!(v.mode.as_str(), "0");
        round_trip::<HkSyn3>("HKSYN:5:3+2'");
    }

    #[test]
    fn builds_the_expected_wire() {
        let built = to_wire(&crate::segments::request::hksyn(), 5, None).unwrap();
        // The expected wire string.
        assert_eq!(built, b"HKSYN:5:3+0'");
        let (_, derived) = from_wire::<HkSyn3>(SAMPLES[0].as_bytes()).unwrap();
        assert_eq!(to_wire(&derived, 5, None).unwrap(), built);
    }

    #[test]
    fn mode_is_one_character() {
        let e = from_wire::<HkSyn3>(b"HKSYN:5:3+00'").unwrap_err();
        assert_eq!(e.field, "mode");
    }
}
