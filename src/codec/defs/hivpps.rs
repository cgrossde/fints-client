//! HIVPPS, Namensabgleich Prüfauftrag Parameter. VoP 1.02, C.10.7.1.1 and
//! the data dictionary.

use crate::codec::{An, Code, DataElementGroup, Jn, Num, Segment};

/// HIVPPS version 1, bank parameter segment. Reference segment HKVVB.
///
/// ```text
/// Nr. Name                           Typ  Format Länge Status Anzahl
///  1  Segmentkopf                    DEG               M     1
///  2  Maximale Anzahl Aufträge       DE   num    ..3   M     1
///  3  Anzahl Signaturen mindestens   DE   num     1    M     1
///  4  Sicherheitsklasse              DE   code    1    M     1
///  5  Parameter Namensabgleich Prüfauftrag DEG v1      M     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIVPPS", version = 1)]
pub struct HiVpps1 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens (0, 1, 2, 3).
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Sicherheitsklasse (0 to 4).
    #[de(len = 1)]
    pub security_class: Code,
    /// 5 Parameter Namensabgleich Prüfauftrag.
    pub parameters: PayeeCheckParameters1,
}

/// Parameter Namensabgleich Prüfauftrag, version 1. VoP 1.02, D.
///
/// ```text
/// Nr. Name                                    Typ  Format Länge Status Anzahl
///  1  Maximale Anzahl CreditTransferTransactionInformation Opt-In
///                                             DE   num    ..7    M     1
///  2  Aufklärungstext strukturiert            DE   jn      #     M     1
///  3  Art der Lieferung Payment Status Report DE   code     1    M     1   S, V
///  4  Sammelzahlungen mit einem Auftrag erlaubt DE jn      #     M     1
///  5  Eingabe Anzahl Einträge erlaubt         DE   jn      #     M     1
///  6  Unterstützte Payment Status Report Datenformate DE an ..1024 M  1
///  7  VOP-pflichtiger Zahlungsverkehrsauftrag DE   an     ..6    M     n
/// ```
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct PayeeCheckParameters1 {
    /// 1 Maximale Anzahl CreditTransferTransactionInformation Opt-In (0:
    /// unlimited).
    #[de(max = 7)]
    pub max_transactions: Num,
    /// 2 Aufklärungstext strukturiert: the explanation text may carry
    /// markup (`<br>`, `<p>`, `<b>`, ...).
    pub explanation_structured: Jn,
    /// 3 Art der Lieferung Payment Status Report: `S` stepwise, `V` complete.
    #[de(len = 1)]
    pub delivery: Code,
    /// 4 Sammelzahlungen mit einem Auftrag erlaubt.
    pub bulk_with_one_order_allowed: Jn,
    /// 5 Eingabe Anzahl Einträge erlaubt.
    pub max_entries_input_allowed: Jn,
    /// 6 Unterstützte Payment Status Report Datenformate (one data element,
    /// several formats separated by `;` per the data dictionary).
    #[de(max = 1024)]
    pub report_formats: An,
    /// 7 VOP-pflichtiger Zahlungsverkehrsauftrag (order codes, upper case).
    #[de(max = 6)]
    pub required_for: Vec<An>,
}

// Synthetic: the shapes banks send (complete delivery with a URN; stepwise
// delivery with a file name).
#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HIVPPS:7:1:4+1+1+0+1000:N:V:N:N:urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.002.001.10:HKCCM:HKCCS:HKIPZ'",
    "HIVPPS:9:1:4+1+1+0+20:J:S:N:N:sepade.pain.002.001.10.xsd:HKIPZ:HKCCS'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HiVpps1 = round_trip(SAMPLES[0]);
        let p = &v.parameters;
        assert_eq!(p.max_transactions, Num::new(1000));
        assert_eq!(p.explanation_structured, Jn::NO);
        assert_eq!(p.delivery.as_str(), "V");
        assert_eq!(
            p.report_formats.as_str(),
            "urn:iso:std:iso:20022:tech:xsd:pain.002.001.10"
        );
        let orders: Vec<&str> = p.required_for.iter().map(An::as_str).collect();
        assert_eq!(orders, ["HKCCM", "HKCCS", "HKIPZ"]);

        let v: HiVpps1 = round_trip(SAMPLES[1]);
        let p = &v.parameters;
        assert_eq!(p.max_transactions, Num::new(20));
        assert_eq!(p.explanation_structured, Jn::YES);
        assert_eq!(p.delivery.as_str(), "S");
        assert_eq!(p.required_for.len(), 2);
    }

    #[test]
    fn no_required_order_is_fine() {
        let v: HiVpps1 = round_trip("HIVPPS:7:1:4+1+1+0+1:N:V:N:N:x'");
        assert!(v.parameters.required_for.is_empty());
    }

    #[test]
    fn the_delivery_is_one_character() {
        let e = from_wire::<HiVpps1>(b"HIVPPS:7:1:4+1+1+0+1:N:VV:N:N:x'").unwrap_err();
        assert_eq!(e.field, "parameters.delivery");
    }
}
