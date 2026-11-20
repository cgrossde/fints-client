//! HIBPA, Bankparameter allgemein (general bank parameters). Formals 2017,
//! D.2.

use super::common::Kik1;
use crate::codec::{An, Code, DataElementGroup, Num, Segment};

/// HIBPA version 3.
///
/// ```text
/// Nr. Name                       Typ  Format Länge Status Anzahl
///  1  Segmentkopf                DEG              M     1
///  2  BPD-Version                DE   num    ..3   M     1
///  3  Kreditinstitutskennung     DEG  kik    #     M     1
///  4  Kreditinstitutsbezeichnung DE   an     ..60  M     1
///  5  Anzahl Geschäftsvorfallsarten DE num   ..3   M     1
///  6  Unterstützte Sprachen      DEG              M     1
///  7  Unterstützte HBCI-Versionen DEG             M     1
///  8  Maximale Nachrichtengröße  DE   num    ..4   O     1
///  9  Minimaler Timeout-Wert     DE   num    ..4   O     1
/// 10  Maximaler Timeout-Wert     DE   num    ..4   O     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIBPA", version = 3)]
pub struct HiBpa3 {
    /// 2 BPD-Version.
    #[de(max = 3)]
    pub bpd_version: Num,
    /// 3 Kreditinstitutskennung.
    pub bank: Kik1,
    /// 4 Kreditinstitutsbezeichnung.
    #[de(max = 60)]
    pub bank_name: An,
    /// 5 Anzahl Geschäftsvorfallsarten: the most kinds of order per
    /// message; `0` means no limit.
    #[de(max = 3)]
    pub max_order_kinds: Num,
    /// 6 Unterstützte Sprachen.
    pub languages: SupportedLanguages2,
    /// 7 Unterstützte HBCI-Versionen.
    pub hbci_versions: SupportedHbciVersions2,
    /// 8 Maximale Nachrichtengröße, in KiB (1024 bytes). The client may not
    /// send a larger message.
    #[de(max = 4)]
    pub max_message_size: Option<Num>,
    /// 9 Minimaler Timeout-Wert.
    #[de(max = 4)]
    pub min_timeout: Option<Num>,
    /// 10 Maximaler Timeout-Wert.
    #[de(max = 4)]
    pub max_timeout: Option<Num>,
}

/// Unterstützte Sprachen, version 2. Formals 2017, data dictionary.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct SupportedLanguages2 {
    /// 1 Unterstützte Sprache, `code ..3`, M, 1 to 9 (1 German, 2 English,
    /// 3 French; see "Dialogsprache").
    #[de(max = 3, min_count = 1, max_count = 9)]
    pub languages: Vec<Code>,
}

/// Unterstützte HBCI-Versionen, version 2. Formals 2017, data dictionary.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct SupportedHbciVersions2 {
    /// 1 Unterstützte HBCI-Version, `num ..3`, M, 1 to 9 (`300` for FinTS 3.0).
    #[de(max = 3, min_count = 1, max_count = 9)]
    pub versions: Vec<Num>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HIBPA:3:3:4+42+280:12345678+Testbank+3+1:2+300+100+10+120'",
    "HIBPA:3:3:4+42+280:12345678+Testbank+0+1:2:3+201:210:220:300'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HiBpa3 = round_trip(SAMPLES[0]);
        assert_eq!(v.bpd_version, Num::new(42));
        assert_eq!(v.bank_name.as_str(), "Testbank");
        assert_eq!(v.languages.languages.len(), 2);
        assert_eq!(v.hbci_versions.versions, vec![Num::new(300)]);
        assert_eq!(
            (v.max_message_size, v.min_timeout, v.max_timeout),
            (Some(Num::new(100)), Some(Num::new(10)), Some(Num::new(120)))
        );
        let v: HiBpa3 = round_trip(SAMPLES[1]);
        assert_eq!(v.hbci_versions.versions.len(), 4);
        assert_eq!(v.max_message_size, None);
    }

    /// The string of the hand-written parser's test reads with the
    /// definition (its bank name is `0`).
    #[test]
    fn reads_the_hand_written_parsers_fixture() {
        let (_, v) = from_wire::<HiBpa3>(b"HIBPA:5:3:4+42+280+0+1+1+0'").unwrap();
        assert_eq!(v.bpd_version, Num::new(42));
        assert_eq!(v.bank.bank_code, None);
    }

    #[test]
    fn rules_are_enforced() {
        // Ten languages are one too many.
        let langs = ["1"; 10].join(":");
        let wire = format!("HIBPA:3:3:4+42+280:12345678+Testbank+3+{langs}+300'");
        let e = from_wire::<HiBpa3>(wire.as_bytes()).unwrap_err();
        assert!(e.field.starts_with("languages"), "{}", e.field);
        let e = from_wire::<HiBpa3>(b"HIBPA:3:3:4+42+280:12345678+Testbank+3++300'").unwrap_err();
        assert!(e.field.starts_with("languages"), "{}", e.field);
    }
}
