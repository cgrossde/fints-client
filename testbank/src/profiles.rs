//! Bank profiles: a [`BankModel`] that behaves like one real bank, written
//! by hand from what was observed.
//!
//! Each rule's doc comment cites its evidence:
//!
//! - **live 2026-11-16**, **live 2026-11-17**: live runs of those days;
//!   the committed traces (`fixtures/replay/<bank>-<BLZ>.trace.json`, from
//!   later runs, their dates moved) show the same and are compared with
//!   the models by `tests/bank_profiles_test.rs`;
//! - **earlier live**: an earlier live run, recorded only in `docs/banks.md`;
//! - **unverified**: `docs/banks.md` only, never seen in a trace.
//!
//! Only public bank parameter data (BPD, return codes, allowed jobs) comes
//! from the bank. Customer, accounts, names and numbers are synthetic.
//! `tests/bank_profiles_test.rs` compares the parameter data with the trace.
//!
//! A profile may carry the real bank code and BIC (public). It must only be
//! used in-process (`TestBank::in_process`), so the code never reaches the
//! network; the binary serves a profile under a synthetic code.

use std::collections::BTreeMap;

use crate::model::{
    Account, BankModel, Booking, CheckOrder, Customer, Delivery, InstantStatusModel, JobSpec,
    LoginSca, OnMatch, Payee, PaymentsModel, ResultAs, TanMethod, TanRule, VopModel, date, iban_de,
};

/// A profile constructor.
pub type Profile = fn() -> BankModel;

/// The profiles, by name (the `--profile` value of `fints-testbank`).
pub fn all() -> &'static [(&'static str, Profile)] {
    &[
        ("consorsbank", consorsbank),
        ("dkb", dkb),
        ("comdirect", comdirect),
    ]
}

/// An empty depot statement: an empty MT535 is a valid answer (live
/// 2026-11-16: HKWPD on the depot got `0020` with `:17B::ACTI//N`).
const EMPTY_MT535: &str = ":16R:GENL\r\n:28E:1/ONLY\r\n:17B::ACTI//N\r\n:16S:GENL";

/// Consorsbank, BLZ 76030080.
///
/// - **Bank:** BLZ 76030080 and BIC CSDBDE71XXX, HIBPA name `Consors`
///   (live 2026-11-16, BPD).
/// - **Login:** [`LoginSca::Exempt`]. Init with `HKTAN:4:HKIDN` got `3076`
///   and a dummy HITAN (live 2026-11-16, trace exchange 2).
/// - **TAN methods:** HITANS 7 with 900 (photoTAN, `SecurePlus TAN
///   Generator`, typed) and 901 (decoupled, `Consorsbank/myPrivateBank App`)
///   (live 2026-11-16, BPD).
/// - **HIPINS:** HKSAL, HKKAZ, HKWPD, HKPRO and HKSPA `N`, HKCCS and HKIPZ
///   `J` (live 2026-11-16, BPD).
/// - **Job versions:** the BPD offers HISALS 3-6, HIKAZS 3-7 and HIWPDS 2-6
///   (live 2026-11-16). The test bank builds parameter segments for versions
///   5-7 only, so the model offers 5 and 6 (HKSAL), 5-7 (HKKAZ) and 5-6
///   (HKWPD): the versions the bank has and the engine can serve.
/// - **HKSAL, HKKAZ:** [`TanRule::RequiredExempt`]. Without HKTAN `9050` +
///   `9075` and no `9800`; with HKTAN `3076` and the data (live 2026-11-16,
///   exchanges 3-6).
/// - **HKWPD:** [`TanRule::RejectHktan`] `9160`, and without HKTAN the order
///   works (`0020`, live 2026-11-16, exchange 15). The `9160` for HKWPD *with*
///   HKTAN is **unverified** (`docs/banks/consorsbank.md`; the trace
///   never sent it).
/// - **HISPAS:** national account allowed, `J:J:J` (live 2026-11-16, BPD).
///   The client then sends the full 6-part KTI, which the bank accepts (live
///   2026-11-16, HKKAZ `0020`/`3010` on every account).
/// - **`kti_requires_national`:** a KTI of only `IBAN:BIC` gets `9010`
///   (earlier live, `docs/banks.md`, HKKAZ 7 with HKTAN, every account).
/// - **UPD version 0:** the bank answers HIUPA with version 0 (usage 0),
///   a UPD for this dialog only (live 2026-11-16, UPD); it is sent in every
///   dialog init.
/// - **Accounts:** no Kontoart on any account (live 2026-11-16, UPD). Three
///   accounts allow HKSAL, HKCCS, HKIPZ, HKKAZ, HKSPA, HKPRO. The depot has
///   no IBAN and no Kontoart and allows only HKWPD and HKPRO (live
///   2026-11-16, UPD).
/// - **[`CheckOrder::TanFirst`], not-allowed code `9380`:** HKSAL on the depot
///   without HKTAN got `9075`; resent with HKTAN `9380 Keine Berechtigung
///   für diese Auftragsart mit diesem Konto` (earlier live, `docs/banks.md`;
///   not in the 2026-11-16 trace, which never sent HKSAL to the depot).
///
/// Customer, accounts, balances and bookings are synthetic.
pub fn consorsbank() -> BankModel {
    let blz = "76030080";
    let private = ["HKSAL", "HKCCS", "HKIPZ", "HKKAZ", "HKSPA", "HKPRO"];
    let account = |number: &str, product: &str| Account {
        kind: None,
        product: product.into(),
        allowed_jobs: private.to_vec(),
        ..Account::giro(blz, number, "Test Kunde")
    };
    let depot = Account {
        iban: None,
        kind: None,
        product: "Test Depot".into(),
        allowed_jobs: vec!["HKWPD", "HKPRO"],
        mt535: Some(EMPTY_MT535.into()),
        balance_date: date(2026, 1, 1),
        ..Account::giro(blz, "0000000900", "Test Kunde")
    };

    let mut jobs = BTreeMap::new();
    let tan_exempt = |versions: &[u16]| JobSpec {
        server_tan: TanRule::RequiredExempt,
        ..JobSpec::new(versions)
    };
    jobs.insert("HKSAL", tan_exempt(&[5, 6]));
    jobs.insert("HKKAZ", tan_exempt(&[5, 6, 7]));
    jobs.insert(
        "HKWPD",
        JobSpec {
            server_tan: TanRule::RejectHktan("9160"),
            ..JobSpec::new(&[5, 6])
        },
    );
    jobs.insert("HKPRO", JobSpec::new(&[]));
    jobs.insert("HKSPA", JobSpec::new(&[]));
    for code in ["HKCCS", "HKIPZ"] {
        jobs.insert(
            code,
            JobSpec {
                hipins_needs_tan: true,
                ..JobSpec::new(&[])
            },
        );
    }

    let mut model = BankModel {
        blz: blz.into(),
        bic: "CSDBDE71XXX".into(),
        name: "Consors".into(),
        customer: Customer {
            user_id: "testuser001".into(),
            pin: "test-pin-1".into(),
            customer_id: "testuser001".into(),
            changed_user_id: None,
        },
        accounts: vec![
            account("0000000101", "Test Tagesgeld"),
            account("0000000102", "Test Verrechnungskonto"),
            account("0000000103", "Test Girokonto"),
            depot,
        ],
        tan_methods: vec![
            TanMethod::typed(900, "SecurePlus TAN Generator", "123456"),
            TanMethod::decoupled(901, "Consorsbank/myPrivateBank App", 2),
        ],
        hitans_version: 7,
        login_sca: LoginSca::Exempt,
        jobs,
        hispas_national: Some(true),
        upd_version: 0,
        not_allowed_code: "9380",
        not_allowed_text: "Keine Berechtigung für diese Auftragsart mit diesem Konto",
        check_order: CheckOrder::TanFirst,
        kti_requires_national: true,
        decoupled_max_polls: 0,
        ..BankModel::default()
    };
    model.enable_payments(PaymentsModel {
        hkccs_formats: CONSORS_PAIN001.to_vec(),
        hkipz_versions: vec![1],
        hkipz_formats: CONSORS_PAIN001.to_vec(),
        tan_codes_on_order: true,
        ..payments_with_registry(blz)
    });
    model
}

/// Consorsbank's pain.001 list for HKCCS (HISPAS) and HKIPZ (HIIPZS), the
/// same in both (live 2026-11-17, BPD).
const CONSORS_PAIN001: [&str; 6] = [
    "urn:iso:std:iso:20022:tech:xsd:pain.001.001.09",
    "urn:iso:std:iso:20022:tech:xsd:pain.001.001.03",
    "urn:iso:std:iso:20022:tech:xsd:pain.001.003.03",
    "urn:iso:std:iso:20022:tech:xsd:pain.001.002.03",
    "urn:swift:xsd:$pain.001.002.02",
    "sepade.pain.001.001.02.xsd",
];

/// The payee registry every profile shares: synthetic payees at the test
/// banks' codes, one in each class of the payee check.
fn payments_with_registry(blz: &str) -> PaymentsModel {
    PaymentsModel {
        payees: vec![
            Payee::new(&iban_de(blz, "0000000501"), "Anna Beispiel"),
            Payee::new(&iban_de(blz, "0000000502"), "Berta Maria Muster"),
        ],
        not_applicable: vec![(iban_de(blz, "0000000503"), "Kein Zahlungskonto".to_string())],
        ..PaymentsModel::default()
    }
}

/// Account jobs of a DKB current account (live 2026-11-20, UPD): the first
/// two also allow HKCCM and HKCME (card top-up orders).
const DKB_GIRO_JOBS: [&str; 33] = [
    "HKBAZ", "HKCAZ", "HKCCS", "HKCDB", "HKCDE", "HKCDL", "HKCDN", "HKCSA", "HKCSB", "HKCSE",
    "HKCSL", "HKDSB", "HKDSW", "HKEKP", "HKFRD", "HKIDA", "HKIPD", "HKIPS", "HKIPT", "HKIPZ",
    "HKKAA", "HKKAZ", "HKKIF", "HKPAE", "HKPOF", "HKQTG", "HKSAL", "HKTIA", "HKTSY", "HKZDF",
    "HKZDL", "HKSPA", "HKTAB",
];

/// DKB, BLZ 12030000.
///
/// - **Bank:** BLZ 12030000, BIC BYLADEM1001, HIBPA name `Deutsche Kreditbank
///   Aktiengesellschaft` (live 2026-11-17, BPD).
/// - **Login:** [`LoginSca::Tan`] with the decoupled method 940 (`DKB App`):
///   `HKTAN:4:HKIDN` got `3955`, the status request `3956` while pending,
///   then `3920` + `0900` (live 2026-11-17, exchanges 2-4). The model
///   approves on the second status request. HITANS 7 gives 999 status
///   requests, 5 s before the first and 3 s between (`decoupled_max_polls`
///   999; the waits stay 0 so tests do not sleep).
/// - **TAN methods:** HITANS 6 and 7; 940 decoupled, 910 `chipTAN manuell`
///   and 913 `chipTAN QR` typed (live 2026-11-17, BPD).
/// - **HIPINS:** HKSAL, HKKAZ and HKCAZ `J`, HKWPD `N` (live 2026-11-17,
///   BPD). Every order went out with HKTAN and was answered `3076` (no
///   TAN), exchanges 5-12: [`TanRule::AsHipins`], the default.
/// - **Job versions:** HISALS 6-8, HIKAZS 6 and 7, HICAZS 1 (camt.052.001.08
///   and .02), HIWPDS 6 (live 2026-11-17, BPD). The client chooses HKSAL 8
///   and HKCAZ 1.
/// - **HISPAS:** national account allowed, `N:J:N` (live 2026-11-17, BPD).
/// - **UPD version 0:** dialog-only (live 2026-11-17, HIUPA).
/// - **Accounts:** four current accounts with Kontoart 1, the first two
///   with HKCCM and HKCME as well, and a credit card with Kontoart 50, an
///   account number and **no IBAN**, allowing HKCAZ, HKKAZ, HKSAL and
///   others (live 2026-11-17, UPD). The client has no IBAN for the card
///   and sends the national-only KTI `::number:sub:280:blz` (HISPAS allows
///   national data; unverified live).
/// - **Bookings:** camt with the bank's reference at entry level
///   (`Ntry/AcctSvcrRef`) on the current accounts' bookings, none on the
///   card's (live 2026-11-17, HICAZ shape).
///
/// - **Payments:** HICCSS 1; HIIPZS 1 and 2, v2 with "conversion to SCT
///   allowed" `N`, formats pain.001.001.09 only; HISPAS formats for HKCCS
///   pain.001.001.03 and .001.09; HIIPSS 1 with minimum wait 5; HIVPPS 1:
///   max 1000, unstructured text, delivery `V`, pain.002.001.10 as URN, the
///   check required for 14 orders; HIVPAS 1. HIPINS: HKCCS and HKIPZ `J`,
///   HKVPP, HKVPA and HKIPS `N` (live 2026-11-17, BPD). Unverified, default
///   path: the result comes as pain.002, a match is followed by HKVPA with
///   the TAN ([`OnMatch::Require`]).
///
/// Customer, accounts, balances and bookings are synthetic.
pub fn dkb() -> BankModel {
    let blz = "12030000";
    let giro = |number: &str, with_cards: bool| {
        let mut jobs = DKB_GIRO_JOBS.to_vec();
        if with_cards {
            jobs.extend(["HKCCM", "HKCME"]);
        }
        Account {
            product: "Test Girokonto".into(),
            allowed_jobs: jobs,
            ..Account::giro(blz, number, "Test Kunde")
        }
    };
    let day = |m, d| date(2026, m, d);
    let first = giro("0000000201", true).with_bookings(
        "500.00".parse().expect("decimal"),
        vec![
            Booking::new(day(3, 2), "-45.67", "Testmiete Maerz")
                .counterparty("Test Vermietung")
                .reference("TESTREF00000000000000000201"),
            Booking::new(day(3, 5), "1200.00", "Testgehalt Maerz")
                .counterparty("Test Arbeitgeber")
                .reference("TESTREF00000000000000000202"),
        ],
    );
    let card = Account {
        iban: None,
        kind: Some(50),
        product: "Test Kreditkarte".into(),
        allowed_jobs: vec![
            "HKCAZ", "HKEKP", "HKFRD", "HKKAA", "HKKAZ", "HKPAE", "HKPOF", "HKQTG", "HKSAL",
            "HKTSY", "HKTAB",
        ],
        ..Account::giro(blz, "5000000000000001", "Test Kunde")
    };

    let mut jobs = BTreeMap::new();
    let with_tan = |versions: &[u16]| JobSpec {
        hipins_needs_tan: true,
        ..JobSpec::new(versions)
    };
    jobs.insert("HKSAL", with_tan(&[6, 7, 8]));
    jobs.insert("HKKAZ", with_tan(&[6, 7]));
    jobs.insert(
        "HKCAZ",
        JobSpec {
            formats: vec![CAMT052_08, CAMT052_02],
            ..with_tan(&[1])
        },
    );
    jobs.insert("HKWPD", JobSpec::new(&[6]));

    let mut model = BankModel {
        blz: blz.into(),
        bic: "BYLADEM1001".into(),
        name: "Deutsche Kreditbank Aktiengesellschaft".into(),
        customer: Customer {
            user_id: "testuser002".into(),
            pin: "test-pin-2".into(),
            customer_id: "testuser002".into(),
            changed_user_id: None,
        },
        accounts: vec![
            first,
            giro("0000000202", true),
            giro("0000000203", false),
            giro("0000000204", false),
            card,
        ],
        tan_methods: vec![
            TanMethod::decoupled(940, "DKB App", 2),
            TanMethod::typed(910, "chipTAN manuell", "123456"),
            TanMethod::typed(913, "chipTAN QR", "123456"),
        ],
        hitans_version: 7,
        hitans6_too: true,
        login_sca: LoginSca::Tan,
        jobs,
        hispas_national: Some(true),
        upd_version: 0,
        decoupled_max_polls: 999,
        ..BankModel::default()
    };
    for code in ["HKVPP", "HKVPA", "HKIPS"] {
        model.jobs.insert(code, JobSpec::new(&[]));
    }
    model.enable_payments(PaymentsModel {
        hkccs_formats: vec![PAIN001_03, PAIN001_09],
        hkipz_versions: vec![1, 2],
        hkipz_formats: vec![PAIN001_09],
        hkipz_conversion_allowed: false,
        hkips: Some(InstantStatusModel {
            min_wait: 5,
            formats: Vec::new(),
        }),
        vop: Some(VopModel {
            max_transactions: 1000,
            structured_text: false,
            delivery: Delivery::Complete,
            psr_formats: vec!["urn:iso:std:iso:20022:tech:xsd:pain.002.001.10"],
            required_orders: vec![
                "HKCCM", "HKCCS", "HKCDE", "HKCDN", "HKCME", "HKCSA", "HKCSE", "HKIDA", "HKIPD",
                "HKIPE", "HKIPM", "HKIPT", "HKIPZ", "HKTIA",
            ],
            result_as: ResultAs::Pain002,
            hkvpa_on_match: OnMatch::Require,
            vpa_answer: Some(("0010", "Nachricht entgegengenommen")),
            ..VopModel::default()
        }),
        ..payments_with_registry(blz)
    });
    model
}

/// pain.001 descriptors as the banks list them.
const PAIN001_03: &str = "urn:iso:std:iso:20022:tech:xsd:pain.001.001.03";
const PAIN001_09: &str = "urn:iso:std:iso:20022:tech:xsd:pain.001.001.09";

/// comdirect, one bank code of 20041111-20041199 (this model: 20041155).
///
/// - **Bank:** BLZ 20041155, BIC COBADEHD055, HIBPA name `comdirect bank`
///   (live 2026-11-17, BPD). The directory has eight codes for one host.
/// - **Login:** [`LoginSca::Exempt`]: `HKTAN:4:HKIDN` got `3076` (live
///   2026-11-17, exchange 2).
/// - **TAN methods:** HITANS 6 with 901 `mobileTAN-Verfahren` and 902
///   `photoTAN-Verfahren`, typed (live 2026-11-17, BPD).
/// - **HIPINS:** HKSAL, HKKAZ and HKCAZ `J`, HKWPD `N` (live 2026-11-17,
///   BPD); HKSAL and HKCAZ with HKTAN got `3076`, HKWPD without HKTAN
///   `0020` (exchanges 3-5).
/// - **Job versions:** HISALS 5, HIKAZS 5, HIWPDS 5, HICAZS 1 with
///   `camt.052.001.02` and `camt.052.001.02.xsd` (live 2026-11-17, BPD).
/// - **HISPAS:** national account allowed `N` (live 2026-11-17, BPD).
/// - **UPD version 1:** stored by the client (live 2026-11-17, HIUPA).
/// - **Accounts:** no Kontoart on either. A clearing account with an IBAN
///   allowing HKSAL, HKKAZ, HKCAZ and others; a depot without IBAN
///   allowing HKWPD, HKSAK, HKISA, HKSSP, HKTAN, HKFRD and HKSPA, but no
///   HKSAL, HKKAZ or HKCAZ (live 2026-11-17, UPD). The client takes the
///   depot from its allowed jobs.
/// - **[`BankModel::foreign_blz_9210`]:** an HKIDN with another bank code
///   of the host got one HIRMG with `9050`, `9800`, `9210` and `9340` (live
///   2026-11-17, earlier run with a code that did not match the user).
///
/// - **Payments:** HICCSS 1; HIIPZS 1 with the purpose codes BENE, BONU,
///   CBFF, CHAR, GOVT, PENS, SALA, SSBE, RENT and pain.001.001.09 and
///   .003.01 (each also with `.xsd`); HISPAS formats for HKCCS
///   pain.001.003.03 only (also with `.xsd`); HIIPSS 1 with minimum wait 99;
///   HIVPPS 1: max 20, structured text, delivery `S`,
///   `sepade.pain.002.001.10.xsd`, the check required for 9 orders; HIVPAS 1.
///   HIPINS: HKCCS and HKIPZ `J`, HKIPS `N`, HKVPP and HKVPA not listed
///   (live 2026-11-17, BPD). Unverified, default path: result as pain.002,
///   HKVPA with the TAN after a match.
///
/// Customer, accounts, balances, bookings and holdings are synthetic.
pub fn comdirect() -> BankModel {
    let blz = "20041155";
    let clearing = Account {
        kind: None,
        product: "Test Verrechnungskonto".into(),
        allowed_jobs: vec![
            "HKSAK", "HKISA", "HKSSP", "HKSAL", "HKKAZ", "HKCAZ", "HKIPM", "HKISS", "HKCUB",
            "HKCUI", "HKIPZ", "HKIPS", "HKCCS", "HKTAN", "HKFRD", "HKSPA",
        ],
        ..Account::giro(blz, "0000000301", "Test Kunde")
    }
    .with_bookings(
        "250.00".parse().expect("decimal"),
        vec![
            Booking::new(date(2026, 3, 3), "-19.90", "Testkauf").counterparty("Test Handel"),
            Booking::new(date(2026, 3, 6), "75.00", "Testeinzahlung"),
        ],
    );
    let depot = Account {
        iban: None,
        kind: None,
        product: "Test Depot".into(),
        allowed_jobs: vec![
            "HKSAK", "HKISA", "HKSSP", "HKWPD", "HKTAN", "HKFRD", "HKSPA",
        ],
        mt535: Some(MT535_ONE_HOLDING.into()),
        ..Account::giro(blz, "0000000302", "Test Kunde")
    };

    let mut jobs = BTreeMap::new();
    let with_tan = |versions: &[u16]| JobSpec {
        hipins_needs_tan: true,
        ..JobSpec::new(versions)
    };
    jobs.insert("HKSAL", with_tan(&[5]));
    jobs.insert("HKKAZ", with_tan(&[5]));
    jobs.insert(
        "HKCAZ",
        JobSpec {
            formats: vec![
                CAMT052_02,
                "urn:iso:std:iso:20022:tech:xsd:camt.052.001.02.xsd",
            ],
            ..with_tan(&[1])
        },
    );
    jobs.insert("HKWPD", JobSpec::new(&[5]));

    let mut model = BankModel {
        blz: blz.into(),
        bic: "COBADEHD055".into(),
        name: "comdirect bank".into(),
        customer: Customer {
            user_id: "testuser003".into(),
            pin: "test-pin-3".into(),
            customer_id: "testuser003".into(),
            changed_user_id: None,
        },
        accounts: vec![clearing, depot],
        tan_methods: vec![
            TanMethod::typed(901, "mobileTAN-Verfahren", "123456"),
            TanMethod::typed(902, "photoTAN-Verfahren", "123456"),
        ],
        hitans_version: 6,
        login_sca: LoginSca::Exempt,
        jobs,
        hispas_national: Some(false),
        upd_version: 1,
        foreign_blz_9210: true,
        ..BankModel::default()
    };
    model.jobs.insert("HKIPS", JobSpec::new(&[]));
    model.enable_payments(PaymentsModel {
        hkccs_formats: vec![
            "urn:iso:std:iso:20022:tech:xsd:pain.001.003.03",
            "urn:iso:std:iso:20022:tech:xsd:pain.001.003.03.xsd",
        ],
        hkipz_versions: vec![1],
        hkipz_formats: vec![
            PAIN001_09,
            "urn:iso:std:iso:20022:tech:xsd:pain.001.001.09.xsd",
            "urn:iso:std:iso:20022:tech:xsd:pain.001.003.01",
            "urn:iso:std:iso:20022:tech:xsd:pain.001.003.01.xsd",
        ],
        hkipz_purpose_codes: Some("BENE,BONU,CBFF,CHAR,GOVT,PENS,SALA,SSBE,RENT"),
        hkips: Some(InstantStatusModel {
            min_wait: 99,
            formats: vec![
                PAIN001_09,
                "urn:iso:std:iso:20022:tech:xsd:pain.001.001.09.xsd",
            ],
        }),
        vop: Some(VopModel {
            max_transactions: 20,
            structured_text: true,
            delivery: Delivery::Stepwise,
            psr_formats: vec!["sepade.pain.002.001.10.xsd"],
            required_orders: vec![
                "HKIPZ", "HKCUI", "HKCCS", "HKCDE", "HKCDN", "HKCSE", "HKCSA", "HKIPM", "HKCCM",
            ],
            result_as: ResultAs::Pain002,
            hkvpa_on_match: OnMatch::Require,
            message_notice: Some(("3905", "Bitte senden Sie die Anfrage erneut.")),
            vpa_answer: Some(("0020", "Auftrag ausgeführt")),
            order_answer_after_vpa: Some(("0010", "Nachricht entgegengenommen")),
            ..VopModel::default()
        }),
        ..payments_with_registry(blz)
    });
    // Observed 2026-11-18: after HKVPA the order ran without a TAN (3076).
    if let Some(job) = model.jobs.get_mut("HKCCS") {
        job.server_tan = TanRule::Exempt;
    }
    model
}

/// camt.052.001.02 and .08 descriptors as the banks list them in HICAZS.
const CAMT052_02: &str = "urn:iso:std:iso:20022:tech:xsd:camt.052.001.02";
const CAMT052_08: &str = "urn:iso:std:iso:20022:tech:xsd:camt.052.001.08";

/// A depot statement with one holding (synthetic security).
const MT535_ONE_HOLDING: &str = ":16R:GENL\r\n:98A::STAT//20260410\r\n:16S:GENL\r\n\
:16R:FIN\r\n:35B:ISIN XX0000TEST01\r\nTESTFONDS WELT\r\n:93B::AGGR//UNIT/40,\r\n:16S:FIN\r\n-";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::iban_de;

    #[test]
    fn profiles_are_named_and_consistent() {
        for (name, build) in all() {
            let m = build();
            assert!(!name.is_empty());
            assert!(!m.accounts.is_empty(), "{name}");
            for a in &m.accounts {
                if let Some(iban) = &a.iban {
                    assert_eq!(*iban, iban_de(&m.blz, &a.number), "{name}");
                }
            }
        }
    }
}
