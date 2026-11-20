//! HIIPZ, SEPA-Instant Payment Zahlung rückmelden. Messages 2022,
//! C.10.2.9.1.

use crate::codec::{An, Code, Segment};

/// HIIPZ version 1, bank answer to HKIPZ 1. Messages 2022, C.10.2.9.1.1.
///
/// ```text
/// Nr. Name                       Typ  Format Länge Status Anzahl
///  1  Segmentkopf                DEG               M     1
///  2  Auftragsidentifikation     DE   an    ..99   M     1
///  3  SEPA-C-Code                DE   code    #    O     1     1, 2, 3, 4
///  4  Status SEPA-Auftrag        DE   code    #    O     1     1 to 9
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIIPZ", version = 1)]
pub struct HiIpz1 {
    /// 2 Auftragsidentifikation.
    #[de(max = 99)]
    pub order_id: An,
    /// 3 SEPA-C-Code (1, 2, 3, 4).
    pub sepa_c_code: Option<Code>,
    /// 4 Status SEPA-Auftrag (1 to 9).
    pub status: Option<Code>,
}

/// HIIPZ version 2, bank answer to HKIPZ 2. Messages 2022, C.10.2.9.1.2.
/// Same layout as version 1; the SEPA-C-Code values are 3 and 4 only.
///
/// ```text
/// Nr. Name                       Typ  Format Länge Status Anzahl
///  1  Segmentkopf                DEG               M     1
///  2  Auftragsidentifikation     DE   an    ..99   M     1
///  3  SEPA-C-Code                DE   code    #    O     1     3, 4
///  4  Status SEPA-Auftrag        DE   code    #    O     1     1 to 9
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIIPZ", version = 2)]
pub struct HiIpz2 {
    /// 2 Auftragsidentifikation.
    #[de(max = 99)]
    pub order_id: An,
    /// 3 SEPA-C-Code (3, 4).
    pub sepa_c_code: Option<Code>,
    /// 4 Status SEPA-Auftrag (1 to 9).
    pub status: Option<Code>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &["HIIPZ:4:1:3+ORDER-0001+3+2'", "HIIPZ:4:2:3+ORDER-0001'"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HiIpz1 = round_trip(SAMPLES[0]);
        assert_eq!(v.order_id.as_str(), "ORDER-0001");
        assert_eq!(v.sepa_c_code.as_ref().map(Code::as_str), Some("3"));
        assert_eq!(v.status.as_ref().map(Code::as_str), Some("2"));
        let v: HiIpz2 = round_trip(SAMPLES[1]);
        assert!(v.sepa_c_code.is_none() && v.status.is_none());
    }

    #[test]
    fn the_order_id_is_mandatory() {
        assert_eq!(
            from_wire::<HiIpz2>(b"HIIPZ:4:2:3'").unwrap_err().field,
            "order_id"
        );
    }
}
