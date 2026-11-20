//! HNVSD, Verschlüsselte Daten (encrypted data). HBCI security 2024, B.5.4.

use crate::codec::{Bin, Segment};

/// HNVSD version 1: the envelope around the signed segments.
///
/// ```text
/// Nr. Name                  Typ  Format Länge Status Anzahl
///  1  Segmentkopf           DEG              M     1
///  2  Daten, verschlüsselt  DE   bin    ..    M     1
/// ```
///
/// PIN/TAN does not encrypt the segments (transport encryption does): the
/// data are the plain segments from HNSHK to HNSHA (PIN/TAN B.9.10).
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HNVSD", version = 1)]
pub struct HnVsd1 {
    /// 2 Daten, verschlüsselt.
    pub data: Bin,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &["HNVSD:999:1+@21@HKEND:3:1+DLG1234567''"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::to_wire;

    #[test]
    fn wire_round_trips() {
        let v: HnVsd1 = round_trip(SAMPLES[0]);
        assert_eq!(v.data.as_bytes(), b"HKEND:3:1+DLG1234567'");
    }

    /// The expected wire string for these values.
    #[test]
    fn builds_the_expected_wire() {
        let inner = b"HKEND:3:1+DLG1234567'".to_vec();
        let built = b"HNVSD:999:1+@21@HKEND:3:1+DLG1234567''".to_vec();
        assert_eq!(
            to_wire(&HnVsd1 { data: Bin(inner) }, 999, None).unwrap(),
            built
        );
    }
}
