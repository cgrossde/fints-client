//! HIWDU, Depotumsätze (depot transactions). Messages 2022, C.4.3.2.

use crate::codec::{Bin, Segment};

/// HIWDU version 5, bank response.
///
/// ```text
/// Nr. Name            Typ  Format Länge Status Anzahl
///  1  Segmentkopf     DEG              M     1
///  2  Umsatzauskunft  DE   bin    ..    M     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIWDU", version = 5)]
pub struct HiWdu5 {
    /// 2 Umsatzauskunft.
    pub statement: Bin,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &["HIWDU:4:5:3+@5@MT536'"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::{ErrorKind, from_wire};

    #[test]
    fn wire_round_trips() {
        let v: HiWdu5 = round_trip(SAMPLES[0]);
        assert_eq!(v.statement.as_bytes(), b"MT536");
    }

    #[test]
    fn rules_are_enforced() {
        let e = from_wire::<HiWdu5>(b"HIWDU:4:5:3+'").unwrap_err();
        assert_eq!(e.field, "statement");
        assert_eq!(e.kind, ErrorKind::Missing);
    }
}
