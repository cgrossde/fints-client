//! HIWPD, Depotaufstellung. HBCI 2.2, VII.4.3.1 (v5); Messages 2022, C.4.3.1 (v6).

use crate::codec::{Bin, Segment};

/// HIWPD version 5, bank response. HBCI 2.2, VII.4.3.1 (MT 535).
///
/// ```text
/// Nr. Name              Typ  Format Länge Status Anzahl
///  1  Segmentkopf       DEG               M     1
///  2  Depotaufstellung  DE   bin    ..    M     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIWPD", version = 5)]
pub struct HiWpd5 {
    /// 2 Depotaufstellung.
    pub statement: Bin,
}

/// HIWPD version 6, bank response.
///
/// ```text
/// Nr. Name              Typ  Format Länge Status Anzahl
///  1  Segmentkopf       DEG               M     1
///  2  Depotaufstellung  DE   bin    ..    M     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIWPD", version = 6)]
pub struct HiWpd6 {
    /// 2 Depotaufstellung.
    pub statement: Bin,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &["HIWPD:4:6:3+@5@MT535'", "HIWPD:4:5:3+@5@MT535'"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HiWpd6 = round_trip(SAMPLES[0]);
        assert_eq!(v.statement.as_bytes(), b"MT535");
        let v: HiWpd5 = round_trip(SAMPLES[1]);
        assert_eq!(v.statement.as_bytes(), b"MT535");
    }

    #[test]
    fn rules_are_enforced() {
        let e = from_wire::<HiWpd6>(b"HIWPD:4:6:3+MT535'").unwrap_err();
        assert_eq!(e.field, "statement");
        assert_eq!(e.kind, crate::codec::ErrorKind::ExpectedBinary);
        let e = from_wire::<HiWpd5>(b"HIWPD:4:5:3+MT535'").unwrap_err();
        assert_eq!(e.field, "statement");
        assert_eq!(e.kind, crate::codec::ErrorKind::ExpectedBinary);
    }
}
