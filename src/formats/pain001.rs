//! pain.001 builder for one SEPA credit transfer (HKCCS) or instant
//! transfer (HKIPZ): docs/payments-design.md section 5.1.
//!
//! Sources: FinTS Messages C.10.2 (one `PmtInf`, one `CdtTrfTxInf`, no
//! `BtchBookg`, `ReqdExctnDt` 1999-01-01), DFÜ-Abkommen Anlage 3 V26.11
//! sections 2.1, 2.2.1 (pain.001.001.09) and 2.2.4.1 (SCT Inst: `LclInstrm`
//! `INST`), and the DK schemas `pain.001.001.09_GBIC_5.xsd` and
//! `pain.001.003.03.xsd` (Anlage 3 V2.9 archive) for element order.
//!
//! The builder is pure and cannot fail: the caller passes checked data
//! ([`Pain001Input`]), the IDs ([`PainIds`]) and the creation time, so the
//! same input gives the same bytes. It knows nothing of the transfer job.

use jiff::{Timestamp, civil::DateTime};
use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use rand::Rng;
use rand::distr::{Alphanumeric, Distribution};
use rust_decimal::Decimal;

use crate::types::{Bic, Iban};

/// The pain.001 versions this client builds.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Pain001Version {
    /// pain.001.001.09 (ISO 2019, DK GBIC_5): preferred.
    V001_001_09,
    /// pain.001.003.03 (DK 2009 variant): fallback.
    V001_003_03,
}

impl Pain001Version {
    /// In order of preference.
    pub const ALL: [Pain001Version; 2] = [Self::V001_001_09, Self::V001_003_03];

    /// `pain.001.001.09` or `pain.001.003.03`.
    pub fn name(self) -> &'static str {
        match self {
            Self::V001_001_09 => "pain.001.001.09",
            Self::V001_003_03 => "pain.001.003.03",
        }
    }

    /// The XML namespace (the URN without `.xsd`).
    pub fn namespace(self) -> &'static str {
        match self {
            Self::V001_001_09 => "urn:iso:std:iso:20022:tech:xsd:pain.001.001.09",
            Self::V001_003_03 => "urn:iso:std:iso:20022:tech:xsd:pain.001.003.03",
        }
    }
}

/// The pain.001 version chosen for one order and the descriptor string as
/// the bank announced it (sent back verbatim in the SEPA descriptor DE).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Pain001Descriptor {
    pub version: Pain001Version,
    pub announced: String,
}

/// Choose the pain.001 version from the descriptors a bank announces for
/// an order (HISPAS for HKCCS; HIIPZS, else HISPAS, for HKIPZ): 001.001.09
/// first, then 001.003.03. The first announced entry that names the
/// version is used (banks list the URN with and without `.xsd`). `None`
/// when no version is in common.
///
/// An instant order (`instant`) needs `LclInstrm` `INST`, which only
/// 001.001.09 has: 001.003.03 is skipped for it (payments design 5.4).
pub(crate) fn choose_descriptor(offered: &[String], instant: bool) -> Option<Pain001Descriptor> {
    Pain001Version::ALL
        .into_iter()
        .filter(|v| !instant || *v != Pain001Version::V001_003_03)
        .find_map(|version| {
            offered
                .iter()
                .find(|d| names_version(d, version))
                .map(|d| Pain001Descriptor {
                    version,
                    announced: d.clone(),
                })
        })
}

/// `urn:iso:std:iso:20022:tech:xsd:pain.001.001.09[.xsd]`: the version
/// name followed by nothing, `.xsd`, or a non-alphanumeric separator.
fn names_version(descriptor: &str, version: Pain001Version) -> bool {
    let name = version.name();
    descriptor.match_indices(name).any(|(i, _)| {
        let rest = &descriptor[i + name.len()..];
        rest.is_empty()
            || rest.starts_with(".xsd")
            || !rest.starts_with(|c: char| c.is_ascii_digit())
    })
}

/// `MsgId` and `PmtInfId` of one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PainIds {
    pub message_id: String,
    pub payment_info_id: String,
}

impl PainIds {
    /// `FC` + UTC `yyyyMMddHHmmss` + 8 random alphanumerics, one draw per
    /// ID (24 characters, SEPA reference set).
    pub(crate) fn generate<R: Rng + ?Sized>(now: Timestamp, rng: &mut R) -> Self {
        let message_id = generate_reference(now, rng);
        let payment_info_id = generate_reference(now, rng);
        Self {
            message_id,
            payment_info_id,
        }
    }
}

/// A unique reference: `FC` + UTC `yyyyMMddHHmmss` + 8 random
/// alphanumerics (24 characters, SEPA reference set). Used for `MsgId`,
/// `PmtInfId` and the generated `EndToEndId`.
pub(crate) fn generate_reference<R: Rng + ?Sized>(now: Timestamp, rng: &mut R) -> String {
    let random: String = (0..8)
        .map(|_| char::from(Alphanumeric.sample(rng)))
        .collect();
    format!("FC{}{random}", now.strftime("%Y%m%d%H%M%S"))
}

/// One checked credit transfer, flat: every field is already valid for
/// the SEPA character sets and lengths (the job checks them first).
#[derive(Debug, Clone)]
pub(crate) struct Pain001Input<'a> {
    pub debtor_name: &'a str,
    pub debtor_iban: &'a Iban,
    /// `None`: `Othr/Id` `NOTPROVIDED` (IBAN-only).
    pub debtor_bic: Option<&'a Bic>,
    pub payee_name: &'a str,
    pub payee_iban: &'a Iban,
    /// `None`: no `CdtrAgt`.
    pub payee_bic: Option<&'a Bic>,
    pub amount: Decimal,
    /// May be empty: no `RmtInf`.
    pub purpose: &'a str,
    /// The caller's, or the generated one.
    pub end_to_end_id: &'a str,
    /// For HKIPZ: `LclInstrm/Cd` `INST`; needs pain.001.001.09
    /// ([`choose_descriptor`] never pairs an instant order with
    /// 001.003.03).
    pub instant: bool,
}

/// A built pain.001 message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Pain001 {
    /// The descriptor to send next to the message (as announced).
    pub descriptor: String,
    pub message_id: String,
    /// UTF-8 XML document.
    pub xml: Vec<u8>,
}

/// `ReqdExctnDt` for FinTS single orders (Messages C.10.2.1): the bank
/// executes at once; any other date is rejected with 9150.
pub(crate) const EXECUTION_DATE: &str = "1999-01-01";

/// Build the message. `created` is `CreDtTm`.
pub(crate) fn build(
    t: &Pain001Input<'_>,
    descriptor: &Pain001Descriptor,
    ids: &PainIds,
    created: DateTime,
) -> Pain001 {
    debug_assert!(
        [&ids.message_id, &ids.payment_info_id]
            .iter()
            .all(|id| super::sepa::is_sepa_reference(id)),
        "pain.001 IDs must be SEPA references: {ids:?}"
    );
    let v09 = descriptor.version == Pain001Version::V001_001_09;
    debug_assert!(v09 || !t.instant, "instant needs pain.001.001.09");
    let bic_tag = if v09 { "BICFI" } else { "BIC" };
    let amount = format_amount(t.amount);

    let mut x = Xml::new();
    x.decl();
    x.open_attr("Document", &[("xmlns", descriptor.version.namespace())]);
    x.open("CstmrCdtTrfInitn");

    x.open("GrpHdr");
    x.leaf("MsgId", &ids.message_id);
    x.leaf(
        "CreDtTm",
        &created.strftime("%Y-%m-%dT%H:%M:%S").to_string(),
    );
    x.leaf("NbOfTxs", "1");
    x.leaf("CtrlSum", &amount);
    x.open("InitgPty");
    x.leaf("Nm", t.debtor_name);
    x.close("InitgPty");
    x.close("GrpHdr");

    x.open("PmtInf");
    x.leaf("PmtInfId", &ids.payment_info_id);
    x.leaf("PmtMtd", "TRF");
    x.leaf("NbOfTxs", "1");
    x.leaf("CtrlSum", &amount);
    x.open("PmtTpInf");
    x.open("SvcLvl");
    x.leaf("Cd", "SEPA");
    x.close("SvcLvl");
    if t.instant && v09 {
        x.open("LclInstrm");
        x.leaf("Cd", "INST");
        x.close("LclInstrm");
    }
    x.close("PmtTpInf");
    if v09 {
        x.open("ReqdExctnDt");
        x.leaf("Dt", EXECUTION_DATE);
        x.close("ReqdExctnDt");
    } else {
        x.leaf("ReqdExctnDt", EXECUTION_DATE);
    }
    x.open("Dbtr");
    x.leaf("Nm", t.debtor_name);
    x.close("Dbtr");
    x.account("DbtrAcct", t.debtor_iban.as_str());
    x.open("DbtrAgt");
    x.open("FinInstnId");
    match t.debtor_bic {
        Some(bic) => x.leaf(bic_tag, bic.as_str()),
        None => {
            x.open("Othr");
            x.leaf("Id", "NOTPROVIDED");
            x.close("Othr");
        }
    }
    x.close("FinInstnId");
    x.close("DbtrAgt");
    x.leaf("ChrgBr", "SLEV");

    x.open("CdtTrfTxInf");
    x.open("PmtId");
    x.leaf("EndToEndId", t.end_to_end_id);
    x.close("PmtId");
    x.open("Amt");
    x.leaf_attr("InstdAmt", &[("Ccy", "EUR")], &amount);
    x.close("Amt");
    if let Some(bic) = t.payee_bic {
        x.open("CdtrAgt");
        x.open("FinInstnId");
        x.leaf(bic_tag, bic.as_str());
        x.close("FinInstnId");
        x.close("CdtrAgt");
    }
    x.open("Cdtr");
    x.leaf("Nm", t.payee_name);
    x.close("Cdtr");
    x.account("CdtrAcct", t.payee_iban.as_str());
    if !t.purpose.is_empty() {
        x.open("RmtInf");
        x.leaf("Ustrd", t.purpose);
        x.close("RmtInf");
    }
    x.close("CdtTrfTxInf");
    x.close("PmtInf");

    x.close("CstmrCdtTrfInitn");
    x.close("Document");

    Pain001 {
        descriptor: descriptor.announced.clone(),
        message_id: ids.message_id.clone(),
        xml: x.finish(),
    }
}

/// Two decimals, `.` as separator (Anlage 3 2.3.3).
fn format_amount(amount: Decimal) -> String {
    let mut a = amount;
    a.rescale(2);
    a.to_string()
}

/// A thin layer over the quick-xml writer; writing into a `Vec` cannot
/// fail, so the I/O results are dropped.
struct Xml(Writer<Vec<u8>>);

impl Xml {
    fn new() -> Self {
        Self(Writer::new_with_indent(Vec::new(), b' ', 2))
    }
    fn event(&mut self, e: Event<'_>) {
        // Infallible for Vec<u8>.
        let _ = self.0.write_event(e);
    }
    fn decl(&mut self) {
        self.event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)));
    }
    fn open(&mut self, name: &str) {
        self.event(Event::Start(BytesStart::new(name)));
    }
    fn open_attr(&mut self, name: &str, attrs: &[(&str, &str)]) {
        let start = BytesStart::new(name).with_attributes(attrs.iter().copied());
        self.event(Event::Start(start));
    }
    fn close(&mut self, name: &str) {
        self.event(Event::End(BytesEnd::new(name)));
    }
    fn leaf(&mut self, name: &str, text: &str) {
        self.leaf_attr(name, &[], text);
    }
    fn leaf_attr(&mut self, name: &str, attrs: &[(&str, &str)], text: &str) {
        self.open_attr(name, attrs);
        self.event(Event::Text(BytesText::new(text)));
        self.close(name);
    }
    fn account(&mut self, tag: &str, iban: &str) {
        self.open(tag);
        self.open("Id");
        self.leaf("IBAN", iban);
        self.close("Id");
        self.close(tag);
    }
    fn finish(self) -> Vec<u8> {
        let mut out = self.0.into_inner();
        out.push(b'\n');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use std::str::FromStr;

    struct Owned {
        debtor_iban: Iban,
        debtor_bic: Option<Bic>,
        payee_iban: Iban,
        payee_bic: Option<Bic>,
        purpose: &'static str,
        end_to_end_id: &'static str,
    }

    fn owned() -> Owned {
        Owned {
            debtor_iban: Iban::parse("DE02120300000000202051").unwrap(),
            debtor_bic: Some(Bic::parse("BYLADEM1001").unwrap()),
            payee_iban: Iban::parse("de89 3704 0044 0532 0130 00").unwrap(),
            payee_bic: Some(Bic::parse("cobadeffxxx").unwrap()),
            purpose: "Rechnung 2026-11 'Test'",
            end_to_end_id: "E2E-TEST-1",
        }
    }

    fn input(o: &Owned, instant: bool) -> Pain001Input<'_> {
        Pain001Input {
            debtor_name: "Erika Beispiel",
            debtor_iban: &o.debtor_iban,
            debtor_bic: o.debtor_bic.as_ref(),
            payee_name: "Max Muster & Söhne",
            payee_iban: &o.payee_iban,
            payee_bic: o.payee_bic.as_ref(),
            amount: Decimal::from_str("1").unwrap(),
            purpose: o.purpose,
            end_to_end_id: o.end_to_end_id,
            instant,
        }
    }

    fn ids() -> PainIds {
        PainIds {
            message_id: "FC20261117091530ABCDEFGH".into(),
            payment_info_id: "FC20261117091530IJKLMNOP".into(),
        }
    }

    fn created() -> DateTime {
        "2026-11-17T10:15:30".parse().unwrap()
    }

    fn descriptor(version: Pain001Version) -> Pain001Descriptor {
        Pain001Descriptor {
            version,
            announced: format!("urn:iso:std:iso:20022:tech:xsd:{}", version.name()),
        }
    }

    fn built(o: &Owned, version: Pain001Version, instant: bool) -> String {
        let p = build(&input(o, instant), &descriptor(version), &ids(), created());
        String::from_utf8(p.xml).unwrap()
    }

    fn golden(name: &str) -> String {
        let path = format!("{}/tests/fixtures/pain/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    /// Golden files; checked once against the DK schemas with xmllint
    /// (docs/payments-design.md section 5.1).
    #[test]
    fn golden_messages() {
        let t = owned();
        let no_bic = Owned {
            debtor_bic: None,
            payee_bic: None,
            purpose: "",
            end_to_end_id: "RE-2026/11",
            ..owned()
        };
        for (file, t, version, instant) in [
            (
                "pain.001.001.09.xml",
                &t,
                Pain001Version::V001_001_09,
                false,
            ),
            (
                "pain.001.001.09.inst.xml",
                &t,
                Pain001Version::V001_001_09,
                true,
            ),
            (
                "pain.001.003.03.xml",
                &t,
                Pain001Version::V001_003_03,
                false,
            ),
            (
                "pain.001.001.09.nobic.xml",
                &no_bic,
                Pain001Version::V001_001_09,
                false,
            ),
            (
                "pain.001.003.03.nobic.xml",
                &no_bic,
                Pain001Version::V001_003_03,
                false,
            ),
        ] {
            let got = built(t, version, instant);
            if std::env::var_os("UPDATE_GOLDEN").is_some() {
                let path = format!("{}/tests/fixtures/pain/{file}", env!("CARGO_MANIFEST_DIR"));
                std::fs::write(path, &got).unwrap();
            }
            assert_eq!(got, golden(file), "{file}");
        }
    }

    #[test]
    fn rules_in_the_message() {
        let x = built(&owned(), Pain001Version::V001_001_09, true);
        assert!(x.contains("<Dt>1999-01-01</Dt>"));
        assert!(x.contains("<Cd>INST</Cd>"));
        assert!(x.contains("<InstdAmt Ccy=\"EUR\">1.00</InstdAmt>"));
        assert!(x.contains("Max Muster &amp; Söhne"));
        assert!(!x.contains("BtchBookg"));
        let x = built(&owned(), Pain001Version::V001_003_03, false);
        assert!(x.contains("<ReqdExctnDt>1999-01-01</ReqdExctnDt>"));
        assert!(x.contains("<BIC>COBADEFFXXX</BIC>"));
    }

    #[test]
    fn amounts() {
        let d = |s: &str| Decimal::from_str(s).unwrap();
        assert_eq!(format_amount(d("1")), "1.00");
        assert_eq!(format_amount(d("1.5")), "1.50");
        assert_eq!(format_amount(d("1.500")), "1.50");
        assert_eq!(format_amount(d("999999999.99")), "999999999.99");
    }

    #[test]
    fn generated_ids() {
        let now = "2026-11-17T09:15:30Z".parse::<Timestamp>().unwrap();
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        let a = PainIds::generate(now, &mut rng);
        assert!(a.message_id.starts_with("FC20261117091530"));
        assert_eq!(a.message_id.len(), 24);
        assert_ne!(a.message_id, a.payment_info_id);
        assert!(super::super::sepa::is_sepa_reference(&a.message_id));
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        assert_eq!(
            PainIds::generate(now, &mut rng),
            a,
            "deterministic per seed"
        );
    }

    /// Descriptor choice per bank BPD (payments-design section 1):
    /// synthetic lists in the shapes the banks announce.
    #[test]
    fn descriptor_choice() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        // Both versions, URN with and without .xsd: the first entry naming
        // the preferred version is sent verbatim.
        let both = s(&[
            "urn:iso:std:iso:20022:tech:xsd:pain.001.003.03",
            "urn:iso:std:iso:20022:tech:xsd:pain.001.001.09.xsd",
            "urn:iso:std:iso:20022:tech:xsd:pain.001.001.09",
        ]);
        let d = choose_descriptor(&both, false).unwrap();
        assert_eq!(d.version, Pain001Version::V001_001_09);
        assert_eq!(
            d.announced,
            "urn:iso:std:iso:20022:tech:xsd:pain.001.001.09.xsd"
        );
        // Only 001.003.03 (one bank's HKCCS formats).
        let old = s(&[
            "urn:iso:std:iso:20022:tech:xsd:pain.001.003.03.xsd",
            "urn:iso:std:iso:20022:tech:xsd:pain.008.003.02.xsd",
        ]);
        let d = choose_descriptor(&old, false).unwrap();
        assert_eq!(d.version, Pain001Version::V001_003_03);
        assert_eq!(d.announced, old[0]);
        // Neither: pain.001.001.03 and other messages only.
        let none = s(&[
            "urn:iso:std:iso:20022:tech:xsd:pain.001.001.03",
            "urn:iso:std:iso:20022:tech:xsd:pain.001.001.090",
            "urn:iso:std:iso:20022:tech:xsd:pain.008.001.08",
        ]);
        assert_eq!(choose_descriptor(&none, false), None);
        assert_eq!(choose_descriptor(&[], false), None);
        // An instant order skips 001.003.03 (no LclInstrm INST).
        assert_eq!(choose_descriptor(&old, true), None);
        let d = choose_descriptor(&both, true).unwrap();
        assert_eq!(d.version, Pain001Version::V001_001_09);
        assert_eq!(
            Pain001Version::V001_001_09.namespace(),
            "urn:iso:std:iso:20022:tech:xsd:pain.001.001.09"
        );
    }
}
