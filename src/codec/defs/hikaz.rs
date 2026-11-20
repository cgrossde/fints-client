//! HIKAZ, Kontoumsätze (booked and pending statement data). HBCI 2.2,
//! VII.2.1.1 (v5); Messages 2022, C.2.1.1.1.1 (v6, v7).

use crate::codec::{Bin, Segment};

/// HIKAZ version 5, bank response. HBCI 2.2, VII.2.1.1. Body identical to versions 6 and 7.
///
/// ```text
/// Nr. Name                    Typ  Format Länge Status Anzahl
///  1  Segmentkopf             DEG               M     1
///  2  Gebuchte Umsätze        DE   bin    ..    M     1
///  3  Nicht gebuchte Umsätze  DE   bin    ..    K     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIKAZ", version = 5)]
pub struct HiKaz5 {
    /// 2 Gebuchte Umsätze.
    pub booked: Bin,
    /// 3 Nicht gebuchte Umsätze.
    pub unbooked: Option<Bin>,
}

/// HIKAZ version 6, bank response. Body identical to version 7.
///
/// ```text
/// Nr. Name                    Typ  Format Länge Status Anzahl
///  1  Segmentkopf             DEG               M     1
///  2  Gebuchte Umsätze        DE   bin    ..    M     1
///  3  Nicht gebuchte Umsätze  DE   bin    ..    O     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIKAZ", version = 6)]
pub struct HiKaz6 {
    /// 2 Gebuchte Umsätze.
    pub booked: Bin,
    /// 3 Nicht gebuchte Umsätze.
    pub unbooked: Option<Bin>,
}

/// HIKAZ version 7, bank response. Same body as version 6.
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIKAZ", version = 7)]
pub struct HiKaz7 {
    /// 2 Gebuchte Umsätze.
    pub booked: Bin,
    /// 3 Nicht gebuchte Umsätze.
    pub unbooked: Option<Bin>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HIKAZ:4:7:3+@5@MT940'",
    "HIKAZ:4:7:3+@5@MT940+@5@MT942'",
    "HIKAZ:4:6:3+@5@MT940'",
    "HIKAZ:4:5:3+@5@MT940'",
    "HIKAZ:4:5:3+@5@MT940+@5@MT942'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::{ErrorKind, from_wire};

    #[test]
    fn wire_round_trips() {
        let v: HiKaz7 = round_trip(SAMPLES[0]);
        assert_eq!(v.booked, Bin(b"MT940".to_vec()));
        assert!(v.unbooked.is_none());

        let v: HiKaz7 = round_trip(SAMPLES[1]);
        assert_eq!(v.booked, Bin(b"MT940".to_vec()));
        assert_eq!(v.unbooked, Some(Bin(b"MT942".to_vec())));

        let v: HiKaz6 = round_trip(SAMPLES[2]);
        assert_eq!(v.booked, Bin(b"MT940".to_vec()));
        assert!(v.unbooked.is_none());

        let v: HiKaz5 = round_trip(SAMPLES[3]);
        assert_eq!(v.booked, Bin(b"MT940".to_vec()));
        assert!(v.unbooked.is_none());
        let v: HiKaz5 = round_trip(SAMPLES[4]);
        assert_eq!(v.unbooked, Some(Bin(b"MT942".to_vec())));
    }

    #[test]
    fn rules_are_enforced() {
        let e = from_wire::<HiKaz7>(b"HIKAZ:4:7:3+MT940'").unwrap_err();
        assert_eq!(e.field, "booked");
        assert_eq!(e.kind, ErrorKind::ExpectedBinary);
        let e = from_wire::<HiKaz5>(b"HIKAZ:4:5:3+MT940'").unwrap_err();
        assert_eq!(e.field, "booked");
        assert_eq!(e.kind, ErrorKind::ExpectedBinary);
    }
}
