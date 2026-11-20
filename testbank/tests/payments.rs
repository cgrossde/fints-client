//! The test bank's payment answers at message level, without the client
//! (the client job does not exist yet): requests are built from the
//! segment definitions and a small synthetic pain.001, the answers are read
//! back with the codec.
//!
//! Each test covers one answer shape of the payee check and payment flow
//! (payments design, section 3.2) or one profile's announced parameters
//! (section 1).

use fints_client::codec::defs::common::Kti1;
use fints_client::codec::defs::{
    HiIpss1, HiIpz1, HiIpz2, HiIpzs1, HiIpzs2, HiPins1, HiRms2, HiTan6, HiTan7, HiVpp1, HiVpps1,
    HkCcs1, HkIps1, HkIpz1, HkIpz2, HkVpa1, HkVpp1, SupportedPainMessages1,
    SupportedPaymentStatusReports1,
};
use fints_client::codec::{self, An, Bin, Jn, ParsedSegment, Segment};
use fints_client_testbank::{
    BankHandle, BankModel, Delivery, InstantStatusModel, JobSpec, LimitAnswer, OnMatch, Payee,
    PaymentsModel, ResultAs, TanMethod, TanRule, TestBank, VopModel, iban_de, latin1, profiles,
};

const BLZ: &str = "12345678";
const USER: &str = "testuser1";
const PIN: &str = "test-pin-1";
const PAIN001_09: &str = "urn:iso:std:iso:20022:tech:xsd:pain.001.001.09";
const PAIN002: &str = "urn:iso:std:iso:20022:tech:xsd:pain.002.001.10";

fn payee_iban() -> String {
    iban_de("87654321", "0000000042")
}
fn payee2_iban() -> String {
    iban_de("87654321", "0000000043")
}
fn na_iban() -> String {
    iban_de("87654321", "0000000044")
}
fn unknown_iban() -> String {
    iban_de("87654321", "0000000045")
}
fn own_iban() -> String {
    iban_de(BLZ, "0001234567")
}

// ---- wire helpers ----------------------------------------------------

/// A segment of a request, written once its number is known.
type Part = Box<dyn Fn(u16) -> Vec<u8>>;

fn typed<T: Segment + Clone + 'static>(v: T) -> Part {
    Box::new(move |n| codec::to_wire(&v, n, None).expect("segment"))
}

fn text(f: impl Fn(u16) -> String + 'static) -> Part {
    Box::new(move |n| f(n).into_bytes())
}

/// A customer message (PIN/TAN F.2.1 layout) of `parts`, numbered from 3.
fn message(
    (blz, user, pin): (&str, &str, &str),
    dialog: &str,
    number: u32,
    function: &str,
    parts: &[Part],
    tan: Option<&str>,
) -> Vec<u8> {
    let profile = if function == "999" { 1 } else { 2 };
    let mut inner = format!(
        "HNSHK:2:4+PIN:{profile}+{function}+4711+1+1+1::0+1+1:20260101:120000+1:999:1+6:10:16+280:{blz}:{user}:S:0:0'"
    )
    .into_bytes();
    for (i, part) in parts.iter().enumerate() {
        inner.extend(part(3 + i as u16));
    }
    let sha = 3 + parts.len();
    let signature = tan.map_or(pin.to_string(), |t| format!("{pin}:{t}"));
    inner.extend(format!("HNSHA:{sha}:2+4711++{signature}'").into_bytes());
    let vsk = format!(
        "HNVSK:998:3+PIN:{profile}+998+1+1::0+1:20260101:120000+2:2:13:@8@\0\0\0\0\0\0\0\0:5:1+280:{blz}:{user}:V:0:0+0'"
    );
    let mut rest = vsk.into_bytes();
    rest.extend(format!("HNVSD:999:1+@{}@", inner.len()).into_bytes());
    rest.extend(inner);
    rest.extend(format!("'HNHBS:{}:1+{number}'", sha + 1).into_bytes());
    let head_len = format!("HNHBK:1:3+{:012}+300+{dialog}+{number}'", 0).len();
    let size = head_len + rest.len();
    let mut out = format!("HNHBK:1:3+{size:012}+300+{dialog}+{number}'").into_bytes();
    out.extend(rest);
    out
}

/// The bank's answer, read back.
#[derive(Debug)]
struct Reply {
    text: String,
    segs: Vec<ParsedSegment>,
}

impl Reply {
    fn parse(answer: &[u8]) -> Reply {
        let marker = b"HNVSD:999:1+@";
        let start = answer
            .windows(marker.len())
            .position(|w| w == marker)
            .expect("HNVSD")
            + marker.len();
        let len_end = start
            + answer[start..]
                .iter()
                .position(|&b| b == b'@')
                .expect("length");
        let len: usize = latin1(&answer[start..len_end]).parse().expect("length");
        let body = &answer[len_end + 1..len_end + 1 + len];
        let segs = codec::parse_message(body).expect("tokenizes");
        for s in &segs {
            if let ParsedSegment::Raw { error, segment } = s {
                // Parameter segments the codec does not read stay raw.
                assert!(
                    segment.segment_type() == "HISPAS" || segment.segment_type().ends_with('S'),
                    "{} unreadable: {error}",
                    segment.segment_type()
                );
            }
        }
        Reply {
            text: latin1(body),
            segs,
        }
    }

    fn all<T: Segment>(&self) -> Vec<T> {
        self.segs
            .iter()
            .filter(|s| s.code() == T::CODE)
            .filter_map(|s| s.to_raw().ok())
            .filter_map(|raw| codec::decode_segment::<T>(&raw).ok().map(|(_, v)| v))
            .collect()
    }

    fn get<T: Segment>(&self) -> T {
        self.all::<T>().into_iter().next().unwrap_or_else(|| {
            panic!("no {} in {}", T::CODE, self.text);
        })
    }

    /// The return codes of the HIRMS that answer segment `reference`.
    fn codes_for(&self, reference: u16) -> Vec<String> {
        self.segs
            .iter()
            .filter_map(|s| match s {
                ParsedSegment::Typed { header, .. }
                    if header.code == "HIRMS" && header.reference == Some(reference) =>
                {
                    s.to_raw().ok()
                }
                _ => None,
            })
            .filter_map(|raw| codec::decode_segment::<HiRms2>(&raw).ok())
            .flat_map(|(_, r)| r.feedback)
            .map(|f| f.code.as_str().to_string())
            .collect()
    }

    fn has(&self, code: &str) -> bool {
        self.segs.iter().any(|s| s.code() == code)
    }
}

/// A dialog of the customer at the test bank.
struct Dlg {
    bank: BankHandle,
    number: u32,
    function: &'static str,
    hktan_version: u16,
    id: String,
}

impl Dlg {
    /// Opens a dialog (no TAN: the login is exempt).
    fn open(bank: &BankHandle, function: &'static str, hktan_version: u16) -> Dlg {
        let init = message(
            (BLZ, USER, PIN),
            "0",
            1,
            function,
            &[
                text(|n| format!("HKIDN:{n}:2+280:{BLZ}+{USER}+0+1'")),
                text(|n| format!("HKVVB:{n}:3+12+4+0+TESTPRODUKT+1.0'")),
            ],
            None,
        );
        bank.exchange(&init).expect("init");
        let id = format!("TESTDLG{:04}", bank.requests().len());
        Dlg {
            bank: bank.clone(),
            number: 1,
            function,
            hktan_version,
            id,
        }
    }

    fn try_send(&mut self, parts: &[Part], tan: Option<&str>) -> Result<Reply, String> {
        self.number += 1;
        let bytes = message(
            (BLZ, USER, PIN),
            &self.id,
            self.number,
            self.function,
            parts,
            tan,
        );
        let answer = self.bank.exchange(&bytes);
        if answer.is_err() {
            // A violation does not advance the bank's message count.
            self.number -= 1;
        }
        answer.map(|a| Reply::parse(&a)).map_err(|v| v.message)
    }

    fn send(&mut self, parts: &[Part]) -> Reply {
        self.try_send(parts, None).unwrap_or_else(|e| panic!("{e}"))
    }

    fn send_tan(&mut self, parts: &[Part], tan: &str) -> Reply {
        self.try_send(parts, Some(tan))
            .unwrap_or_else(|e| panic!("{e}"))
    }

    /// HKTAN process 4 for `order`.
    fn tan4(&self, order: &'static str) -> Part {
        let v = self.hktan_version;
        text(move |n| format!("HKTAN:{n}:{v}+4+{order}'"))
    }

    /// HKTAN process 2 (typed) or S (decoupled) for `task`.
    fn tan_next(&self, task: &'static str) -> Part {
        let (v, p) = (
            self.hktan_version,
            if self.function == "940" { "S" } else { "2" },
        );
        text(move |n| format!("HKTAN:{n}:{v}+{p}++++{task}+N'"))
    }
}

fn an(s: &str) -> An {
    An::new(s).expect("an")
}

fn kti(iban: &str) -> Kti1 {
    Kti1 {
        iban: Some(an(iban)),
        bic: Some(an("TESTDEFFXXX")),
        account_number: None,
        subaccount: None,
        bank: None,
    }
}

fn ccs(pain: &[u8]) -> Part {
    typed(HkCcs1 {
        account: kti(&own_iban()),
        descriptor: an(PAIN001_09),
        message: Bin(pain.to_vec()),
    })
}

fn ipz(pain: &[u8], version: u16, convert: bool) -> Part {
    if version == 1 {
        typed(HkIpz1 {
            account: kti(&own_iban()),
            descriptor: an(PAIN001_09),
            message: Bin(pain.to_vec()),
        })
    } else {
        typed(HkIpz2 {
            account: kti(&own_iban()),
            descriptor: an(PAIN001_09),
            message: Bin(pain.to_vec()),
            convert_to_sepa_allowed: Some(Jn(convert)),
        })
    }
}

fn vpp(descriptor: &str, poll: Option<(&str, &str)>) -> Part {
    typed(HkVpp1 {
        reports: SupportedPaymentStatusReports1 {
            descriptors: vec![an(descriptor)],
        },
        polling_id: poll.map(|(id, _)| Bin(id.as_bytes().to_vec())),
        max_entries: None,
        touchdown: poll.map(|(_, td)| an(td)),
    })
}

fn vpa(id: &[u8]) -> Part {
    typed(HkVpa1 {
        vop_id: Bin(id.to_vec()),
    })
}

/// A pain.001.001.09 of one transfer, as the client would send it (only
/// the elements the bank reads matter).
fn pain(msg_id: &str, payee: &str, iban: &str, amount: &str, date: &str, inst: bool) -> Vec<u8> {
    let instant = if inst {
        "<LclInstrm><Cd>INST</Cd></LclInstrm>"
    } else {
        ""
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<Document xmlns=\"urn:iso:std:iso:20022:tech:xsd:pain.001.001.09\"><CstmrCdtTrfInitn>\
<GrpHdr><MsgId>{msg_id}</MsgId><CreDtTm>2026-01-01T12:00:00</CreDtTm><NbOfTxs>1</NbOfTxs>\
<CtrlSum>{amount}</CtrlSum><InitgPty><Nm>Test Kunde</Nm></InitgPty></GrpHdr>\
<PmtInf><PmtInfId>PMT-{msg_id}</PmtInfId><PmtMtd>TRF</PmtMtd><NbOfTxs>1</NbOfTxs>\
<PmtTpInf><SvcLvl><Cd>SEPA</Cd></SvcLvl>{instant}</PmtTpInf>\
<ReqdExctnDt><Dt>{date}</Dt></ReqdExctnDt><Dbtr><Nm>Test Kunde</Nm></Dbtr>\
<DbtrAcct><Id><IBAN>{own}</IBAN></Id></DbtrAcct>\
<DbtrAgt><FinInstnId><BICFI>TESTDEFFXXX</BICFI></FinInstnId></DbtrAgt><ChrgBr>SLEV</ChrgBr>\
<CdtTrfTxInf><PmtId><EndToEndId>E2E-{msg_id}</EndToEndId></PmtId>\
<Amt><InstdAmt Ccy=\"EUR\">{amount}</InstdAmt></Amt>\
<Cdtr><Nm>{payee}</Nm></Cdtr><CdtrAcct><Id><IBAN>{iban}</IBAN></Id></CdtrAcct>\
</CdtTrfTxInf></PmtInf></CstmrCdtTrfInitn></Document>",
        own = own_iban()
    )
    .into_bytes()
}

fn good_pain(msg_id: &str) -> Vec<u8> {
    pain(
        msg_id,
        "Anna Beispiel",
        &payee_iban(),
        "1.00",
        "1999-01-01",
        false,
    )
}

// ---- models -----------------------------------------------------------

fn payments(vop: Option<VopModel>) -> PaymentsModel {
    PaymentsModel {
        hkips: Some(InstantStatusModel {
            min_wait: 5,
            formats: Vec::new(),
        }),
        payees: vec![
            Payee::new(&payee_iban(), "Anna Beispiel"),
            Payee::new(&payee2_iban(), "Berta Maria Muster"),
        ],
        not_applicable: vec![(na_iban(), "Kein Zahlungskonto".into())],
        vop,
        ..PaymentsModel::default()
    }
}

fn vop(on_match: OnMatch) -> VopModel {
    VopModel {
        hkvpa_on_match: on_match,
        ..VopModel::default()
    }
}

/// A typed-TAN bank (function 921, HKTAN 6) with these payments.
fn typed_bank(p: PaymentsModel) -> BankHandle {
    let mut model = BankModel {
        hispas_national: Some(false),
        ..BankModel::default()
    };
    model.enable_payments(p);
    TestBank::new(model).handle()
}

/// A decoupled bank (function 940, HKTAN 7, approved on the second poll).
fn decoupled_bank(p: PaymentsModel) -> BankHandle {
    let mut model = BankModel {
        tan_methods: vec![TanMethod::decoupled(940, "Test-App", 2)],
        hitans_version: 7,
        ..BankModel::default()
    };
    model.enable_payments(p);
    TestBank::new(model).handle()
}

fn typed_dialog(p: PaymentsModel) -> (BankHandle, Dlg) {
    let bank = typed_bank(p);
    let d = Dlg::open(&bank, "921", 6);
    (bank, d)
}

const TAN: &str = "123456";

fn vop_id(r: &Reply) -> Vec<u8> {
    r.get::<HiVpp1>().vop_id.expect("VOP-ID").0
}

/// Message 1 for the default payee: `HKVPP + HKCCS + HKTAN:4`, numbered
/// 3, 4, 5.
fn message_1(d: &mut Dlg, pain: &[u8]) -> Reply {
    let parts = [vpp(PAIN002, None), ccs(pain), d.tan4("HKCCS")];
    d.send(&parts)
}

// ---- answer shapes ----------------------------------------------------

#[test]
fn match_with_3091_ends_with_the_tan_and_no_hkvpa() {
    let (bank, mut d) = typed_dialog(payments(Some(vop(OnMatch::Skip3091))));
    let pain = good_pain("MSG1");
    let r = message_1(&mut d, &pain);
    assert_eq!(r.codes_for(3), ["0025", "3091"]);
    assert_eq!(r.codes_for(5), ["0030"]);
    // HITAN 6: process 4, no hash, the order reference, the challenge.
    let hitan = r.get::<HiTan6>();
    assert_eq!(hitan.order_reference.expect("task").as_str(), "TASK0001");
    let v = r.get::<HiVpp1>();
    assert!(v.vop_id.is_some() && v.polling_id.is_none());
    let report = String::from_utf8(v.report.expect("pain.002").0).unwrap();
    assert!(report.contains("<TxSts>RCVC</TxSts>"), "{report}");
    assert_eq!(v.report_descriptor.expect("descriptor").as_str(), PAIN002);
    assert!(bank.payments().is_empty(), "nothing before the TAN");

    // A wrong TAN executes nothing, the right one executes exactly once.
    let wrong = d
        .try_send(&[d.tan_next("TASK0001")], Some("000000"))
        .unwrap();
    assert_eq!(wrong.codes_for(3), ["9941"]);
    assert!(bank.payments().is_empty());
    let done = d.send_tan(&[d.tan_next("TASK0001")], TAN);
    assert_eq!(done.codes_for(3), ["0020"]);
    let paid = bank.payments();
    assert_eq!(paid.len(), 1);
    assert_eq!(paid[0].order, "HKCCS");
    assert_eq!(paid[0].msg_id, "MSG1");
    assert_eq!(paid[0].payee_iban, payee_iban());
    assert_eq!(paid[0].amount.to_string(), "1.00");
    assert!(!paid[0].instant);
    bank.assert_clean();
}

#[test]
fn match_with_3091_decoupled_executes_on_the_approval_poll() {
    let bank = decoupled_bank(payments(Some(vop(OnMatch::Skip3091))));
    let mut d = Dlg::open(&bank, "940", 7);
    let r = message_1(&mut d, &good_pain("MSG1"));
    assert_eq!(r.codes_for(3), ["0025", "3091"]);
    assert_eq!(r.codes_for(5), ["3955"]);
    assert_eq!(r.get::<HiTan7>().process.as_str(), "4");
    let pending = d.send(&[d.tan_next("TASK0001")]);
    assert_eq!(pending.codes_for(3), ["3956"]);
    assert!(bank.payments().is_empty());
    let done = d.send(&[d.tan_next("TASK0001")]);
    assert_eq!(done.codes_for(3), ["0020"]);
    assert_eq!(bank.payments().len(), 1);
    bank.assert_clean();
}

#[test]
fn match_without_3091_wants_hkvpa_with_the_tan() {
    let (bank, mut d) = typed_dialog(payments(Some(vop(OnMatch::Require))));
    let r = message_1(&mut d, &good_pain("MSG1"));
    assert_eq!(r.codes_for(3), ["0025"]);
    assert_eq!(r.codes_for(5), ["0030"]);
    let id = vop_id(&r);
    // The TAN alone is a violation: the bank waits for the HKVPA.
    let err = d
        .try_send(&[d.tan_next("TASK0001")], Some(TAN))
        .unwrap_err();
    assert!(err.contains("HKVPA"), "{err}");
    // An HKVPA with another VOP-ID is refused, the order is not executed.
    let wrong = d
        .try_send(&[vpa(b"OTHER"), d.tan_next("TASK0001")], Some(TAN))
        .unwrap();
    assert_eq!(wrong.codes_for(3), ["9010"]);
    assert!(bank.payments().is_empty());
    let done = d
        .try_send(&[vpa(&id), d.tan_next("TASK0001")], Some(TAN))
        .unwrap();
    assert_eq!(done.codes_for(4), ["0020"]);
    assert_eq!(bank.payments().len(), 1);
    assert_eq!(bank.payments()[0].vop_id, Some(id));
}

#[test]
fn match_without_3091_decoupled_sends_hkvpa_on_the_first_poll_only() {
    let bank = decoupled_bank(payments(Some(vop(OnMatch::Require))));
    let mut d = Dlg::open(&bank, "940", 7);
    let r = message_1(&mut d, &good_pain("MSG1"));
    let id = vop_id(&r);
    let first = d.send(&[vpa(&id), d.tan_next("TASK0001")]);
    assert_eq!(first.codes_for(4), ["3956"]);
    let second = d.send(&[d.tan_next("TASK0001")]);
    assert_eq!(second.codes_for(3), ["0020"]);
    assert_eq!(bank.payments().len(), 1);
    bank.assert_clean();
}

#[test]
fn match_with_3945_asks_for_the_order_again_with_hkvpa() {
    let (bank, mut d) = typed_dialog(payments(Some(vop(OnMatch::Always3945))));
    let pain = good_pain("MSG1");
    let r = message_1(&mut d, &pain);
    assert_eq!(r.codes_for(3), ["0025"]);
    assert_eq!(r.codes_for(5), ["3945"]);
    assert!(!r.has("HITAN"), "no challenge with 3945");
    let id = vop_id(&r);
    // Message 3.3: HKVPA + the same order + a new HKTAN:4.
    let parts = [vpa(&id), ccs(&pain), d.tan4("HKCCS")];
    let r2 = d.send(&parts);
    assert_eq!(r2.codes_for(5), ["0030"]);
    assert!(r2.has("HITAN"));
    assert!(bank.payments().is_empty());
    let done = d.send_tan(&[d.tan_next("TASK0001")], TAN);
    assert_eq!(done.codes_for(3), ["0020"]);
    assert_eq!(bank.payments().len(), 1);
    bank.assert_clean();
}

#[test]
fn deviations_answer_3090_and_3945_with_the_result() {
    // (payee name, IBAN, status, registered name, RVNA reason)
    let cases = [
        ("anna beispiel", payee_iban(), "RCVC", None, None),
        (
            "Beispiel, Anna",
            payee_iban(),
            "RVMC",
            Some("Anna Beispiel"),
            None,
        ),
        (
            "Berta Muster",
            payee2_iban(),
            "RVMC",
            Some("Berta Maria Muster"),
            None,
        ),
        ("Berta Maria Muster", payee2_iban(), "RCVC", None, None),
        ("Hans Falsch", payee_iban(), "RVNM", None, None),
        ("Anna Beispiel", unknown_iban(), "RVNM", None, None),
        (
            "Anna Beispiel",
            na_iban(),
            "RVNA",
            None,
            Some("Kein Zahlungskonto"),
        ),
    ];
    for (i, (name, iban, status, registered, reason)) in cases.into_iter().enumerate() {
        let model = VopModel {
            result_as: ResultAs::SingleDeg,
            ..vop(OnMatch::Skip3091)
        };
        let (bank, mut d) = typed_dialog(payments(Some(model)));
        let msg = format!("MSG{i}");
        let r = message_1(
            &mut d,
            &pain(&msg, name, &iban, "1.00", "1999-01-01", false),
        );
        let result = r.get::<HiVpp1>().result.expect("single-transaction result");
        assert_eq!(result.outcome.as_str(), status, "{name} {iban}");
        assert_eq!(result.iban.as_str(), iban);
        assert_eq!(result.registered_name.as_ref().map(An::as_str), registered);
        assert_eq!(result.rvna_reason.as_ref().map(An::as_str), reason);
        if status == "RCVC" {
            assert_eq!(r.codes_for(3), ["0025", "3091"]);
        } else {
            assert_eq!(r.codes_for(3), ["3090"], "{name}");
            assert_eq!(r.codes_for(5), ["3945"], "{name}");
            assert!(!r.has("HITAN"));
        }
        assert!(bank.payments().is_empty());
    }
}

#[test]
fn close_match_as_pain002_names_the_registered_name() {
    let (_, mut d) = typed_dialog(payments(Some(vop(OnMatch::Require))));
    let r = message_1(
        &mut d,
        &pain(
            "M1",
            "Berta Muster",
            &payee2_iban(),
            "1.00",
            "1999-01-01",
            false,
        ),
    );
    let report = String::from_utf8(r.get::<HiVpp1>().report.unwrap().0).unwrap();
    for part in [
        "<OrgnlMsgId>M1</OrgnlMsgId>",
        "<DtldNbOfTxs>1</DtldNbOfTxs><DtldSts>RVMC</DtldSts>",
        "<OrgnlEndToEndId>E2E-M1</OrgnlEndToEndId><TxSts>RVMC</TxSts>",
        "<StsRsnInf><AddtlInf>Berta Maria Muster</AddtlInf></StsRsnInf>",
        "<Cdtr><Pty><Nm>Berta Muster</Nm></Pty></Cdtr>",
    ] {
        assert!(report.contains(part), "{part} in {report}");
    }
    let (_, mut d) = typed_dialog(payments(Some(vop(OnMatch::Require))));
    let r = message_1(
        &mut d,
        &pain(
            "M2",
            "Anna Beispiel",
            &na_iban(),
            "1.00",
            "1999-01-01",
            false,
        ),
    );
    let report = String::from_utf8(r.get::<HiVpp1>().report.unwrap().0).unwrap();
    assert!(
        report.contains("<TxSts>RVNA</TxSts><StsRsnInf><Rsn><Cd>AG03</Cd></Rsn><AddtlInf>Kein Zahlungskonto</AddtlInf></StsRsnInf>"),
        "{report}"
    );
}

#[test]
fn confirming_a_deviation_sends_hkvpa_and_the_same_order_then_the_tan() {
    let model = VopModel {
        explanation: Some("Der Name weicht ab.".into()),
        ..vop(OnMatch::Require)
    };
    let (bank, mut d) = typed_dialog(payments(Some(model)));
    let pain = pain(
        "M1",
        "Hans Falsch",
        &payee_iban(),
        "1.00",
        "1999-01-01",
        false,
    );
    let r = message_1(&mut d, &pain);
    assert_eq!(r.codes_for(3), ["3090"]);
    assert_eq!(r.codes_for(5), ["3945"]);
    assert_eq!(
        r.get::<HiVpp1>().explanation.expect("text").as_str(),
        "Der Name weicht ab."
    );
    let id = vop_id(&r);
    let parts = [vpa(&id), ccs(&pain), d.tan4("HKCCS")];
    let r2 = d.send(&parts);
    assert_eq!(r2.codes_for(5), ["0030"]);
    // After a confirmation the TAN needs no HKVPA.
    let done = d.send_tan(&[d.tan_next("TASK0001")], TAN);
    assert_eq!(done.codes_for(3), ["0020"]);
    assert_eq!(bank.payments().len(), 1);
    assert_eq!(bank.payments()[0].payee_name, "Hans Falsch");
    bank.assert_clean();
}

#[test]
fn cancelling_at_the_confirmation_executes_nothing() {
    let (bank, mut d) = typed_dialog(payments(Some(vop(OnMatch::Require))));
    let r = message_1(
        &mut d,
        &pain(
            "M1",
            "Hans Falsch",
            &payee_iban(),
            "1.00",
            "1999-01-01",
            false,
        ),
    );
    assert_eq!(r.codes_for(3), ["3090"]);
    // The client sends nothing more; the dialog ends.
    let end = d.send(&[text(|n| format!("HKEND:{n}:1+TESTDLG0001'"))]);
    assert_eq!(end.codes_for(3), ["0020"]);
    assert!(bank.payments().is_empty());
}

#[test]
fn polling_with_complete_delivery_answers_the_result_only_at_the_end() {
    let model = VopModel {
        pending_polls: 2,
        wait_secs: 3,
        delivery: Delivery::Complete,
        ..vop(OnMatch::Require)
    };
    let (bank, mut d) = typed_dialog(payments(Some(model)));
    let pain = good_pain("MSG1");
    let r = message_1(&mut d, &pain);
    assert_eq!(r.codes_for(3), ["3093", "3040"]);
    assert!(
        r.codes_for(5).is_empty(),
        "the HKTAN of message 1 stays unanswered"
    );
    assert!(!r.has("HITAN"));
    let v = r.get::<HiVpp1>();
    assert_eq!(v.polling_id.as_ref().unwrap().0, b"POLL0001");
    assert_eq!(v.wait.unwrap().value(), 3);
    assert!(v.vop_id.is_none() && v.report.is_none() && v.result.is_none());
    assert!(
        r.text
            .contains("3040::Es liegen weitere Informationen vor:VP-1")
    );

    // A poll with another touchdown point is a violation.
    let err = d
        .try_send(&[vpp(PAIN002, Some(("POLL0001", "VP-9")))], None)
        .unwrap_err();
    assert!(err.contains("touchdown"), "{err}");

    let p1 = d.send(&[vpp(PAIN002, Some(("POLL0001", "VP-1")))]);
    assert_eq!(p1.codes_for(3), ["3093", "3040"]);
    assert!(p1.text.contains(":VP-2'") || p1.text.contains(":VP-2+"));
    let p2 = d.send(&[vpp(PAIN002, Some(("POLL0001", "VP-2")))]);
    assert_eq!(p2.codes_for(3), ["3094", "0025"]);
    let id = vop_id(&p2);
    let report = String::from_utf8(p2.get::<HiVpp1>().report.unwrap().0).unwrap();
    assert!(report.contains("<TxSts>RCVC</TxSts>"));

    // Then message 3.3, the TAN and the execution.
    let parts = [vpa(&id), ccs(&pain), d.tan4("HKCCS")];
    let r3 = d.send(&parts);
    assert_eq!(r3.codes_for(5), ["0030"]);
    d.send_tan(&[d.tan_next("TASK0001")], TAN);
    assert_eq!(bank.payments().len(), 1);
}

#[test]
fn polling_with_stepwise_delivery_sends_partial_pain002_parts() {
    let model = VopModel {
        pending_polls: 1,
        delivery: Delivery::Stepwise,
        ..vop(OnMatch::Require)
    };
    let (_, mut d) = typed_dialog(payments(Some(model)));
    let pain = pain(
        "M1",
        "Berta Muster",
        &payee2_iban(),
        "1.00",
        "1999-01-01",
        false,
    );
    let r = message_1(&mut d, &pain);
    let part = String::from_utf8(r.get::<HiVpp1>().report.expect("part").0).unwrap();
    assert!(part.contains("<TxSts>PDNG</TxSts>"), "{part}");
    let p1 = d.send(&[vpp(PAIN002, Some(("POLL0001", "VP-1")))]);
    assert_eq!(p1.codes_for(3), ["3094", "3090"]);
    let last = String::from_utf8(p1.get::<HiVpp1>().report.unwrap().0).unwrap();
    assert!(last.contains("<OrgnlEndToEndId>E2E-M1</OrgnlEndToEndId><TxSts>RVMC</TxSts>"));
    assert_eq!(p1.get::<HiVpp1>().wait, None);
}

#[test]
fn polling_with_the_single_transaction_deg_marks_the_part_pending() {
    let model = VopModel {
        pending_polls: 1,
        delivery: Delivery::Stepwise,
        result_as: ResultAs::SingleDeg,
        ..vop(OnMatch::Require)
    };
    let (_, mut d) = typed_dialog(payments(Some(model)));
    let r = message_1(&mut d, &good_pain("M1"));
    assert_eq!(r.get::<HiVpp1>().result.unwrap().outcome.as_str(), "PDNG");
    let p1 = d.send(&[vpp(PAIN002, Some(("POLL0001", "VP-1")))]);
    assert_eq!(p1.get::<HiVpp1>().result.unwrap().outcome.as_str(), "RCVC");
}

#[test]
fn hkvpa_must_name_a_known_vop_id_and_resend_the_same_bytes() {
    let (bank, mut d) = typed_dialog(payments(Some(vop(OnMatch::Always3945))));
    let pain = good_pain("MSG1");
    let r = message_1(&mut d, &pain);
    let id = vop_id(&r);
    let unknown = d.send(&[vpa(b"NOPE"), ccs(&pain), d.tan4("HKCCS")]);
    assert_eq!(unknown.codes_for(3), ["9010"]);
    assert!(unknown.text.contains("VOP-ID unbekannt"));
    // One byte differs (the creation time): the order is refused.
    let changed: Vec<u8> = String::from_utf8(pain.clone())
        .unwrap()
        .replace("12:00:00", "12:00:01")
        .into_bytes();
    let diff = d.send(&[vpa(&id), ccs(&changed), d.tan4("HKCCS")]);
    assert_eq!(diff.codes_for(4), ["9010"]);
    assert!(diff.text.contains("Auftrag weicht vom Ursprungsauftrag ab"));
    assert!(!diff.has("HITAN"));
    // The same bytes pass; after the execution the VOP-ID is used up.
    let ok = d.send(&[vpa(&id), ccs(&pain), d.tan4("HKCCS")]);
    assert_eq!(ok.codes_for(5), ["0030"]);
    d.send_tan(&[d.tan_next("TASK0001")], TAN);
    assert_eq!(bank.payments().len(), 1);
    let again = d.send(&[vpa(&id), ccs(&pain), d.tan4("HKCCS")]);
    assert_eq!(again.codes_for(3), ["9210"]);
    assert_eq!(bank.payments().len(), 1);
}

#[test]
fn execution_date_other_than_1999_01_01_is_9150() {
    let (bank, mut d) = typed_dialog(payments(Some(vop(OnMatch::Require))));
    let r = message_1(
        &mut d,
        &pain(
            "M1",
            "Anna Beispiel",
            &payee_iban(),
            "1.00",
            "2026-12-01",
            false,
        ),
    );
    assert_eq!(r.codes_for(4), ["9150"]);
    assert!(!r.has("HIVPP") && !r.has("HITAN"));
    assert!(bank.payments().is_empty());
}

#[test]
fn a_repeated_message_id_is_9390() {
    let (bank, mut d) = typed_dialog(payments(None));
    let pain = good_pain("SAME");
    let first = d.send(&[ccs(&pain), d.tan4("HKCCS")]);
    assert_eq!(first.codes_for(4), ["0030"]);
    d.send_tan(&[d.tan_next("TASK0001")], TAN);
    assert_eq!(bank.payments().len(), 1);
    let second = d.send(&[ccs(&pain), d.tan4("HKCCS")]);
    assert_eq!(second.codes_for(3), ["9390"]);
    assert!(!second.has("HITAN"));
    assert_eq!(bank.payments().len(), 1);
}

#[test]
fn without_hivpps_the_order_goes_straight_to_the_tan() {
    let (bank, mut d) = typed_dialog(payments(None));
    let r = d.send(&[ccs(&good_pain("M1")), d.tan4("HKCCS")]);
    assert_eq!(r.codes_for(4), ["0030"]);
    assert!(!r.has("HIVPP"));
    // HKVPP although the BPD has no HIVPPS is a violation.
    let err = d
        .try_send(
            &[vpp(PAIN002, None), ccs(&good_pain("M2")), d.tan4("HKCCS")],
            None,
        )
        .unwrap_err();
    assert!(err.contains("HIVPPS"), "{err}");
    d.send_tan(&[d.tan_next("TASK0001")], TAN);
    assert_eq!(bank.payments().len(), 1);
    assert_eq!(bank.payments()[0].vop_id, None);
}

#[test]
fn a_required_check_without_hkvpp_is_9076_and_no_hktan_is_9075() {
    let (bank, mut d) = typed_dialog(payments(Some(vop(OnMatch::Require))));
    let r = d.send(&[ccs(&good_pain("M1")), d.tan4("HKCCS")]);
    assert_eq!(r.codes_for(3), ["9076"]);
    let r = d.send(&[vpp(PAIN002, None), ccs(&good_pain("M2"))]);
    assert_eq!(r.codes_for(4), ["9075"]);
    assert!(bank.payments().is_empty());
}

#[test]
fn a_waived_tan_executes_the_order_at_once() {
    let mut model = BankModel::default();
    model.enable_payments(payments(Some(vop(OnMatch::Skip3091))));
    model.jobs.get_mut("HKCCS").unwrap().server_tan = TanRule::Exempt;
    let bank = TestBank::new(model).handle();
    let mut d = Dlg::open(&bank, "921", 6);
    let r = message_1(&mut d, &good_pain("M1"));
    assert_eq!(r.codes_for(3), ["0025", "3091"]);
    assert_eq!(r.codes_for(4), ["0020"]);
    assert_eq!(r.codes_for(5), ["3076"]);
    assert_eq!(bank.payments().len(), 1);
}

// ---- instant payments ---------------------------------------------------

fn instant_pain(msg_id: &str, amount: &str) -> Vec<u8> {
    pain(
        msg_id,
        "Anna Beispiel",
        &payee_iban(),
        amount,
        "1999-01-01",
        true,
    )
}

#[test]
fn an_instant_payment_above_the_limit_is_9255_with_a_link_or_9210() {
    for (answer, code, text) in [
        (LimitAnswer::Code9255, "9255", "testbank.invalid/limit"),
        (
            LimitAnswer::Code9210,
            "9210",
            "Betrag zu groß für Instant Payment Zahlung",
        ),
    ] {
        let p = PaymentsModel {
            hkipz_versions: vec![1, 2],
            instant_limit: Some("100.00".parse().unwrap()),
            instant_limit_answer: answer,
            ..payments(None)
        };
        let (bank, mut d) = typed_dialog(p);
        let r = d.send(&[
            ipz(&instant_pain("M1", "100.01"), 1, false),
            d.tan4("HKIPZ"),
        ]);
        assert_eq!(r.codes_for(3), [code]);
        assert!(r.text.contains(text), "{}", r.text);
        assert!(!r.has("HITAN"));
        // Within the limit the order runs.
        let ok = d.send(&[
            ipz(&instant_pain("M2", "100.00"), 1, false),
            d.tan4("HKIPZ"),
        ]);
        assert_eq!(ok.codes_for(4), ["0030"]);
        d.send_tan(&[d.tan_next("TASK0001")], TAN);
        assert_eq!(bank.payments().len(), 1);
        assert!(bank.payments()[0].instant);
    }
}

#[test]
fn hkipz_2_converts_above_the_limit_with_3270_when_allowed() {
    let p = PaymentsModel {
        hkipz_versions: vec![1, 2],
        hkipz_conversion_allowed: true,
        instant_limit: Some("100.00".parse().unwrap()),
        ..payments(None)
    };
    let (bank, mut d) = typed_dialog(p);
    let pain = instant_pain("M1", "500.00");
    let r = d.send(&[ipz(&pain, 2, true), d.tan4("HKIPZ")]);
    assert_eq!(r.codes_for(4), ["0030"]);
    let done = d.send_tan(&[d.tan_next("TASK0001")], TAN);
    assert_eq!(done.codes_for(3), ["0020", "3270"]);
    assert!(done.has("HIIPZ"));
    let paid = bank.payments();
    assert_eq!(paid.len(), 1);
    assert!(paid[0].converted && !paid[0].instant);
    // Without the conversion flag the same order is rejected.
    let r = d.send(&[
        ipz(&instant_pain("M2", "500.00"), 2, false),
        d.tan4("HKIPZ"),
    ]);
    assert_eq!(r.codes_for(3), ["9255"]);
}

#[test]
fn instant_status_is_3045_until_hkips_returns_hiips() {
    let p = PaymentsModel {
        hkipz_versions: vec![1, 2],
        instant_status_after: 2,
        ..payments(None)
    };
    let (bank, mut d) = typed_dialog(p);
    d.send(&[ipz(&instant_pain("M1", "1.00"), 2, false), d.tan4("HKIPZ")]);
    let done = d.send_tan(&[d.tan_next("TASK0001")], TAN);
    assert_eq!(done.codes_for(3), ["0020", "3045"]);
    let order = done.get::<HiIpz2>();
    assert_eq!(order.order_id.as_str(), "INST0001");
    assert_eq!(order.status.unwrap().as_str(), "3");
    let ips = |d: &Dlg| {
        let _ = d;
        typed(HkIps1 {
            account: kti(&own_iban()),
            formats: SupportedPainMessages1 {
                descriptors: vec![an(PAIN001_09)],
            },
            order_id: an("INST0001"),
        })
    };
    let q1 = d.send(&[ips(&d)]);
    assert_eq!(q1.codes_for(3), ["3045"]);
    assert!(!q1.has("HIIPS"));
    let q2 = d.send(&[ips(&d)]);
    assert_eq!(q2.codes_for(3), ["0020"]);
    assert_eq!(
        q2.get::<fints_client::codec::defs::HiIps1>()
            .status
            .unwrap()
            .as_str(),
        "7"
    );
    assert_eq!(bank.payments().len(), 1);
    bank.assert_clean();
}

#[test]
fn instant_status_with_a_tan_answers_with_the_tan() {
    let p = PaymentsModel {
        hkipz_versions: vec![1, 2],
        instant_status_after: 2,
        ..payments(None)
    };
    let mut model = BankModel {
        hispas_national: Some(false),
        ..BankModel::default()
    };
    model.jobs.insert(
        "HKIPS",
        JobSpec {
            hipins_needs_tan: true,
            ..JobSpec::new(&[])
        },
    );
    model.enable_payments(p);
    let bank = TestBank::new(model).handle();
    let mut d = Dlg::open(&bank, "921", 6);
    d.send(&[ipz(&instant_pain("M1", "1.00"), 2, false), d.tan4("HKIPZ")]);
    d.send_tan(&[d.tan_next("TASK0001")], TAN);
    let ips = |d: &Dlg| {
        let part = typed(HkIps1 {
            account: kti(&own_iban()),
            formats: SupportedPainMessages1 {
                descriptors: vec![an(PAIN001_09)],
            },
            order_id: an("INST0001"),
        });
        [part, d.tan4("HKIPS")]
    };
    // First query: 3045 comes with the TAN answer.
    let q1 = d.send(&ips(&d));
    assert!(q1.has("HITAN") && !q1.has("HIIPS"));
    let a1 = d.send_tan(&[d.tan_next("TASK0002")], TAN);
    assert!(a1.codes_for(3).contains(&"3045".to_string()) && !a1.has("HIIPS"));
    // Second query: the status comes with the TAN answer.
    d.send(&ips(&d));
    let a2 = d.send_tan(&[d.tan_next("TASK0003")], TAN);
    assert_eq!(
        a2.get::<fints_client::codec::defs::HiIps1>()
            .status
            .unwrap()
            .as_str(),
        "7"
    );
    bank.assert_clean();
}

#[test]
fn instant_without_a_status_wait_is_final_at_once() {
    let (bank, mut d) = typed_dialog(payments(None));
    d.send(&[ipz(&instant_pain("M1", "1.00"), 1, false), d.tan4("HKIPZ")]);
    let done = d.send_tan(&[d.tan_next("TASK0001")], TAN);
    assert_eq!(done.codes_for(3), ["0020"]);
    assert_eq!(done.get::<HiIpz1>().status.unwrap().as_str(), "7");
    assert_eq!(bank.payments()[0].order_id.as_deref(), Some("INST0001"));
}

// ---- BPD per profile ----------------------------------------------------

/// The BPD a profile sends to a client that has none.
fn profile_bpd(model: BankModel) -> Reply {
    let (blz, user, pin) = (
        model.blz.clone(),
        model.customer.user_id.clone(),
        model.customer.pin.clone(),
    );
    let bank = TestBank::new(model).handle();
    let init = message(
        (&blz, &user, &pin),
        "0",
        1,
        "999",
        &[
            text({
                let (blz, user) = (blz.clone(), user.clone());
                move |n| format!("HKIDN:{n}:2+280:{blz}+{user}+0+1'")
            }),
            text(|n| format!("HKVVB:{n}:3+0+0+0+TESTPRODUKT+1.0'")),
            text(|n| format!("HKSYN:{n}:3+0'")),
        ],
        None,
    );
    Reply::parse(&bank.exchange(&init).expect("sync"))
}

fn hipins(r: &Reply) -> Vec<(String, bool)> {
    r.get::<HiPins1>()
        .parameters
        .orders
        .iter()
        .map(|o| (o.segment_code.as_str().to_string(), o.tan_required.0))
        .collect()
}

fn pin_of(orders: &[(String, bool)], code: &str) -> Option<bool> {
    orders.iter().find(|(c, _)| c == code).map(|(_, j)| *j)
}

#[test]
fn dkb_announces_what_the_trace_shows() {
    let r = profile_bpd(profiles::dkb());
    assert!(r.has("HICCSS"));
    let v1 = r.get::<HiIpzs1>();
    assert_eq!(v1.parameters.formats.len(), 1);
    assert_eq!(v1.parameters.formats[0].as_str(), PAIN001_09);
    let v2 = r.get::<HiIpzs2>();
    assert!(
        !v2.parameters.conversion_allowed.0,
        "conversion to SCT is N"
    );
    assert_eq!(v2.parameters.formats[0].as_str(), PAIN001_09);
    let ips = r.get::<HiIpss1>();
    assert_eq!(ips.parameters.min_wait.value(), 5);
    let vpps = r.get::<HiVpps1>().parameters;
    assert_eq!(vpps.max_transactions.value(), 1000);
    assert!(!vpps.explanation_structured.0);
    assert_eq!(vpps.delivery.as_str(), "V");
    assert_eq!(vpps.report_formats.as_str(), PAIN002);
    let required: Vec<&str> = vpps.required_for.iter().map(An::as_str).collect();
    assert_eq!(required.len(), 14);
    assert!(required.contains(&"HKCCS") && required.contains(&"HKIPZ"));
    assert!(r.has("HIVPAS"));
    // HISPAS: pain.001.001.03 and .09 for HKCCS.
    assert!(
        r.text
            .contains("pain.001.001.03:urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.001.09")
    );
    let pins = hipins(&r);
    for (code, tan) in [
        ("HKCCS", true),
        ("HKIPZ", true),
        ("HKVPP", false),
        ("HKVPA", false),
        ("HKIPS", false),
    ] {
        assert_eq!(pin_of(&pins, code), Some(tan), "{code}");
    }
}

#[test]
fn comdirect_announces_what_the_trace_shows() {
    let r = profile_bpd(profiles::comdirect());
    assert!(r.has("HICCSS"));
    assert!(r.all::<HiIpzs2>().is_empty(), "HKIPZ 1 only");
    let ipz = r.get::<HiIpzs1>().parameters;
    assert_eq!(
        ipz.purpose_codes.unwrap().as_str(),
        "BENE,BONU,CBFF,CHAR,GOVT,PENS,SALA,SSBE,RENT"
    );
    let formats: Vec<&str> = ipz.formats.iter().map(An::as_str).collect();
    assert_eq!(formats.len(), 4);
    assert!(formats.iter().any(|f| f.ends_with("pain.001.003.01")));
    assert_eq!(r.get::<HiIpss1>().parameters.min_wait.value(), 99);
    let vpps = r.get::<HiVpps1>().parameters;
    assert_eq!(vpps.max_transactions.value(), 20);
    assert!(vpps.explanation_structured.0);
    assert_eq!(vpps.delivery.as_str(), "S");
    assert_eq!(vpps.report_formats.as_str(), "sepade.pain.002.001.10.xsd");
    assert_eq!(vpps.required_for.len(), 9);
    assert!(r.has("HIVPAS"));
    // HKCCS: pain.001.003.03 only.
    assert!(r.text.contains("pain.001.003.03"));
    let hispas = r.text.split("HISPAS").nth(1).unwrap();
    assert!(!hispas.contains("pain.001.001"), "{hispas}");
    let pins = hipins(&r);
    assert_eq!(pin_of(&pins, "HKCCS"), Some(true));
    assert_eq!(pin_of(&pins, "HKIPZ"), Some(true));
    assert_eq!(pin_of(&pins, "HKIPS"), Some(false));
    assert_eq!(pin_of(&pins, "HKVPP"), None);
    assert_eq!(pin_of(&pins, "HKVPA"), None);
}

#[test]
fn consorsbank_announces_no_payee_check() {
    let r = profile_bpd(profiles::consorsbank());
    assert!(r.has("HICCSS"));
    let ipz = r.get::<HiIpzs1>().parameters;
    assert_eq!(ipz.formats.len(), 6);
    assert!(r.all::<HiIpzs2>().is_empty());
    assert!(!r.has("HIVPPS") && !r.has("HIVPAS") && !r.has("HIIPSS"));
    assert!(r.text.contains("pain.001.002.03"));
    let pins = hipins(&r);
    assert_eq!(pin_of(&pins, "HKCCS"), Some(true));
    assert_eq!(pin_of(&pins, "HKIPZ"), Some(true));
    for code in ["HKVPP", "HKVPA", "HKIPS"] {
        assert_eq!(pin_of(&pins, code), None, "{code}");
    }
}

#[test]
fn a_profile_takes_a_payment_with_a_comdirect_pain_003_03() {
    // HKCCS on comdirect: the descriptor is pain.001.003.03, the date sits
    // directly in ReqdExctnDt (no <Dt>).
    let model = profiles::comdirect();
    let blz = model.blz.clone();
    let (user, pin) = (model.customer.user_id.clone(), model.customer.pin.clone());
    let iban = model.accounts[0].iban.clone().unwrap();
    let payee = model.payments.as_ref().unwrap().payees[0].clone();
    let bank = TestBank::new(model).handle();
    let creds = (blz.as_str(), user.as_str(), pin.as_str());
    let init = message(
        creds,
        "0",
        1,
        "901",
        &[
            text({
                let (blz, user) = (blz.clone(), user.clone());
                move |n| format!("HKIDN:{n}:2+280:{blz}+{user}+0+1'")
            }),
            text(|n| format!("HKVVB:{n}:3+99+99+0+TESTPRODUKT+1.0'")),
        ],
        None,
    );
    bank.exchange(&init).expect("init");
    let xml = format!(
        "<Document xmlns=\"urn:iso:std:iso:20022:tech:xsd:pain.001.003.03\"><CstmrCdtTrfInitn>\
<GrpHdr><MsgId>C1</MsgId></GrpHdr><PmtInf><PmtInfId>P1</PmtInfId><ReqdExctnDt>1999-01-01</ReqdExctnDt>\
<CdtTrfTxInf><PmtId><EndToEndId>E1</EndToEndId></PmtId><Amt><InstdAmt Ccy=\"EUR\">2.50</InstdAmt></Amt>\
<Cdtr><Nm>{}</Nm></Cdtr><CdtrAcct><Id><IBAN>{}</IBAN></Id></CdtrAcct></CdtTrfTxInf></PmtInf>\
</CstmrCdtTrfInitn></Document>",
        payee.name, payee.iban
    );
    let hkccs = typed(HkCcs1 {
        account: kti(&iban),
        descriptor: an("urn:iso:std:iso:20022:tech:xsd:pain.001.003.03"),
        message: Bin(xml.into_bytes()),
    });
    let vpp = typed(HkVpp1 {
        reports: SupportedPaymentStatusReports1 {
            descriptors: vec![an("sepade.pain.002.001.10.xsd")],
        },
        polling_id: None,
        max_entries: None,
        touchdown: None,
    });
    let tan = text(|n| format!("HKTAN:{n}:6+4+HKCCS'"));
    let r = Reply::parse(
        &bank
            .exchange(&message(
                creds,
                "TESTDLG0001",
                2,
                "901",
                &[vpp, hkccs, tan],
                None,
            ))
            .expect("message 1"),
    );
    assert_eq!(r.codes_for(3), ["0025"]);
    // HKCCS is exempt: a clean match runs without a TAN (unobserved live).
    assert_eq!(r.codes_for(5), ["3076"]);
    let report = String::from_utf8(r.get::<HiVpp1>().report.unwrap().0).unwrap();
    assert!(report.contains("<TxSts>RCVC</TxSts>"));
    bank.assert_clean();
}
