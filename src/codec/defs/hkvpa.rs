//! HKVPA, Namensabgleich Ausführungsauftrag (Verification of Payee,
//! execution order). VoP 1.02, C.10.7.1.2.

use crate::codec::{Bin, Segment};

/// HKVPA version 1, customer order. VoP 1.02, C.10.7.1.2. Confirms the
/// result of the payee check; sent with the original payment order (the
/// pain message again, unchanged).
///
/// ```text
/// Nr. Name                       Typ  Format Länge Status Anzahl
///  1  Segmentkopf                DEG               M     1
///  2  VOP-ID                     DEG  bin    ..    M     1
/// ```
///
/// The table types the VOP-ID as DEG with format `bin`; the data dictionary
/// defines it as a data element `bin` (D, "VOP-ID"), which is what is
/// written here.
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKVPA", version = 1)]
pub struct HkVpa1 {
    /// 2 VOP-ID (the value from HIVPP).
    pub vop_id: Bin,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &["HKVPA:5:1+@5@vop-1'", "HKVPA:5:1+@8@a+b:c?'d'"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HkVpa1 = round_trip(SAMPLES[0]);
        assert_eq!(v.vop_id.0, b"vop-1");
        // Binary data may hold the separators.
        let v: HkVpa1 = round_trip(SAMPLES[1]);
        assert_eq!(v.vop_id.0, b"a+b:c?'d");
    }

    #[test]
    fn the_vop_id_is_binary() {
        assert!(from_wire::<HkVpa1>(b"HKVPA:5:1+text'").is_err());
    }
}
