//! Self-tests of the test bank at message level, without the client.
//!
//! Requests and expected answers are wire strings written by hand from the
//! specification (Formals B; PIN/TAN F.2 example dialog and the segment
//! formats of PIN/TAN and Messages), not produced by the codec. The bank
//! builds its answers with the codec, so a codec error would show up here
//! as a difference instead of hiding on both sides.

use fints_client::codec::{self, ParsedSegment};
use fints_client_testbank::{BankHandle, BankModel, LoginSca, TestBank, iban_de, latin1};

const BLZ: &str = "12345678";
const USER: &str = "testuser1";
const PIN: &str = "test-pin-1";

/// A customer message: HNHBK, HNVSK, HNVSD(HNSHK, `segments`, HNHBS),
/// signed with `function` (PIN/TAN F.2.1 layout).
fn customer(
    dialog: &str,
    number: u32,
    function: &str,
    system_id: &str,
    segments: &[&str],
) -> Vec<u8> {
    signed(dialog, number, function, system_id, segments, None)
}

/// [`customer`] with a TAN in HNSHA.
fn signed(
    dialog: &str,
    number: u32,
    function: &str,
    system_id: &str,
    segments: &[&str],
    tan: Option<&str>,
) -> Vec<u8> {
    let profile = if function == "999" { 1 } else { 2 };
    let shk = format!(
        "HNSHK:2:4+PIN:{profile}+{function}+4711+1+1+1::{system_id}+1+1:20260101:120000+1:999:1+6:10:16+280:{BLZ}:{USER}:S:0:0'"
    );
    let mut inner = shk;
    for s in segments {
        inner.push_str(s);
    }
    let sha_number = 3 + segments.len();
    let signature = match tan {
        Some(tan) => format!("{PIN}:{tan}"),
        None => PIN.to_string(),
    };
    inner.push_str(&format!("HNSHA:{sha_number}:2+4711++{signature}'"));
    let vsk = format!(
        "HNVSK:998:3+PIN:{profile}+998+1+1::{system_id}+1:20260101:120000+2:2:13:@8@\0\0\0\0\0\0\0\0:5:1+280:{BLZ}:{USER}:V:0:0+0'"
    );
    let rest = format!(
        "{vsk}HNVSD:999:1+@{}@{inner}'HNHBS:{}:1+{number}'",
        inner.len(),
        sha_number + 1
    );
    let head_without_size = format!("HNHBK:1:3++300+{dialog}+{number}'");
    let size = head_without_size.len() + 12 + rest.len();
    format!("HNHBK:1:3+{size:012}+300+{dialog}+{number}'{rest}").into_bytes()
}

/// The clear text inside the bank's HNVSD.
fn inner(answer: &[u8]) -> String {
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
    let len: usize = latin1(&answer[start..len_end])
        .parse()
        .expect("length digits");
    latin1(&answer[len_end + 1..len_end + 1 + len])
}

fn sync(bank: &BankHandle) -> Vec<u8> {
    bank.exchange(&customer(
        "0",
        1,
        "999",
        "0",
        &[
            &format!("HKIDN:3:2+280:{BLZ}+{USER}+0+1'"),
            "HKVVB:4:3+0+0+0+TESTPRODUKT+1.0'",
            "HKSYN:5:3+0'",
        ],
    ))
    .expect("sync answer")
}

fn open(bank: &BankHandle, hktan: bool) -> Vec<u8> {
    let mut segments = vec![
        format!("HKIDN:3:2+280:{BLZ}+{USER}+TESTSYS0001+1'"),
        "HKVVB:4:3+12+4+0+TESTPRODUKT+1.0'".to_string(),
    ];
    if hktan {
        segments.push("HKTAN:5:6+4+HKIDN'".into());
    }
    let refs: Vec<&str> = segments.iter().map(String::as_str).collect();
    bank.exchange(&customer("0", 1, "921", "TESTSYS0001", &refs))
        .expect("init answer")
}

#[test]
fn sync_answer_matches_the_hand_written_message() {
    let bank = TestBank::new(BankModel::default()).handle();
    let answer = sync(&bank);
    let iban = iban_de(BLZ, "0001234567");
    let inner = [
        "HIRMG:2:2+3060::Bitte beachten Sie die enthaltenen Warnungen/Hinweise'",
        "HIRMS:3:2:4+0020::Auftrag ausgeführt+3920::Zugelassene Zwei-Schritt-Verfahren für den Benutzer:921'",
        "HIRMS:4:2:5+0020::Auftrag ausgeführt'",
        "HIBPA:5:3:4+12+280:12345678+Testbank+3+1+300'",
        "HITANS:6:6:4+1+1+1+J:N:0:921:2:TEST921:HHD:1.4:Test-TAN:6:1:TAN:6:N:1:N:0:0:N:N:00:0:N:0'",
        "HIPINS:7:1:4+1+1+0+5:20:6:Benutzerkennung:Kunden-ID:HKKAZ:N:HKSAL:N:HKWPD:N'",
        "HIKAZS:8:5:4+1+1+365:J:N'",
        "HIKAZS:9:6:4+1+1+1+365:J:N'",
        "HIKAZS:10:7:4+1+1+1+365:J:N'",
        "HISALS:11:5:4+1+1'",
        "HISALS:12:6:4+1+1+1'",
        "HISALS:13:7:4+1+1+1'",
        "HIWPDS:14:5:4+1+1+N:N:N'",
        "HIWPDS:15:6:4+1+1+1+N:N:N'",
        "HIUPA:16:4:4+testuser1+4+0'",
        &format!(
            "HIUPD:17:6:4+0001234567::280:12345678+{iban}+testuser1+1+EUR+Testkundin Eins++Girokonto++HKSAL:1+HKKAZ:1'"
        ),
        "HISYN:18:4:5+TESTSYS0001'",
    ]
    .concat();
    let rest = format!(
        "HNVSK:998:3+PIN:1+998+1+1::0+1:20260101:120000+2:2:13:@8@\0\0\0\0\0\0\0\0:5:1+280:12345678:testuser1:V:0:0+0'\
         HNVSD:999:1+@{}@{inner}'HNHBS:19:1+1'",
        inner.chars().count()
    );
    let head_without_size = "HNHBK:1:3++300+TESTDLG0001+1+TESTDLG0001:1'";
    let size = head_without_size.len() + 12 + rest.chars().count();
    let expected = format!("HNHBK:1:3+{size:012}+300+TESTDLG0001+1+TESTDLG0001:1'{rest}");
    assert_eq!(latin1(&answer), expected);
    assert_eq!(bank.issued_system_ids(), ["TESTSYS0001"]);
    bank.assert_clean();
}

#[test]
fn balance_answer_matches_the_hand_written_segments() {
    let bank = TestBank::new(BankModel::default()).handle();
    sync(&bank);
    let opened = inner(&open(&bank, true));
    // PIN/TAN B.4.2: an SCA exemption carries a dummy HITAN.
    assert_eq!(
        opened,
        "HIRMG:2:2+3060::Bitte beachten Sie die enthaltenen Warnungen/Hinweise'\
         HIRMS:3:2:4+0020::Auftrag ausgeführt+3920::Zugelassene Zwei-Schritt-Verfahren für den Benutzer:921'\
         HIRMS:4:2:5+3076::Starke Kundenauthentifizierung nicht notwendig'\
         HITAN:5:6:5+4++noref+nochallenge'"
    );
    let iban = iban_de(BLZ, "0001234567");
    let answer = bank
        .exchange(&customer(
            "TESTDLG0001",
            2,
            "921",
            "TESTSYS0001",
            &[&format!("HKSAL:3:7+{iban}:TESTDEFFXXX+N'")],
        ))
        .unwrap_err();
    assert!(
        answer
            .message
            .contains("before the dialog is authenticated"),
        "{answer}"
    );

    let answer = bank
        .exchange(&customer(
            "TESTDLG0002",
            2,
            "921",
            "TESTSYS0001",
            &[&format!("HKSAL:3:7+{iban}:TESTDEFFXXX+N'")],
        ))
        .expect("balance");
    // Messages B: HISAL 7 = Kontoverbindung international, Kontoproduktbezeichnung,
    // Kontowährung, Gebuchter Saldo (Soll-Haben-Kennzeichen:Betrag:Währung:Datum).
    assert_eq!(
        inner(&answer),
        format!(
            "HIRMG:2:2+0010::Nachricht entgegengenommen'\
             HIRMS:3:2:3+0020::Auftrag ausgeführt'\
             HISAL:4:7:3+{iban}:TESTDEFFXXX+Girokonto+EUR+C:1654,33:EUR:20260305'"
        )
    );
    let end = bank
        .exchange(&customer(
            "TESTDLG0002",
            3,
            "921",
            "TESTSYS0001",
            &["HKEND:3:1+TESTDLG0002'"],
        ))
        .expect("end");
    // PIN/TAN F.2.5 b.
    assert_eq!(
        inner(&end),
        "HIRMG:2:2+0100::Dialog beendet'HIRMS:3:2:3+0020::Auftrag ausgeführt'"
    );
    // The first balance request went to the synchronisation dialog.
    assert_eq!(bank.violations().len(), 1);
}

#[test]
fn tan_challenge_matches_the_hand_written_segments() {
    let bank = TestBank::new(BankModel {
        login_sca: LoginSca::Tan,
        ..BankModel::default()
    })
    .handle();
    sync(&bank);
    // PIN/TAN HITAN 6: TAN-Prozess, Auftrags-Hashwert, Auftragsreferenz, Challenge.
    assert_eq!(
        inner(&open(&bank, true)),
        "HIRMG:2:2+3060::Bitte beachten Sie die enthaltenen Warnungen/Hinweise'\
         HIRMS:3:2:4+0020::Auftrag ausgeführt+3920::Zugelassene Zwei-Schritt-Verfahren für den Benutzer:921'\
         HIRMS:4:2:5+0030::Auftrag empfangen - Sicherheitsfreigabe erforderlich'\
         HITAN:5:6:5+4++TASK0001+Bitte geben Sie die TAN ein'"
    );
    // Wrong TAN, then the right one (process 2 with the order reference).
    let submit = |number: u32, tan: &str| {
        bank.exchange(&signed(
            "TESTDLG0002",
            number,
            "921",
            "TESTSYS0001",
            &["HKTAN:3:6+2++++TASK0001+N'"],
            Some(tan),
        ))
        .expect("TAN answer")
    };
    assert_eq!(
        inner(&submit(2, "999999")),
        "HIRMG:2:2+9050::Die Nachricht enthält Fehler'HIRMS:3:2:3+9941::TAN ungültig'"
    );
    assert_eq!(
        inner(&submit(3, "123456")),
        "HIRMG:2:2+0010::Nachricht entgegengenommen'\
         HIRMS:3:2:3+0020::Auftrag ausgeführt'\
         HITAN:4:6:3+2++TASK0001'"
    );
    bank.assert_clean();
}

/// Wrong TANs by [`WrongTanModel`]: 9941 with 9360 (RM: lock after %1
/// further failed attempts, P1) while the login is kept, then 9941 with
/// 9210 (PIN/TAN B.5.2: order deleted) or with HIRMG 9800 (Formals C.1.2:
/// dialog aborted).
#[test]
fn wrong_tan_answers_match_the_hand_written_segments() {
    use fints_client_testbank::{WrongTanEnd, WrongTanModel};
    for (end, last) in [
        (
            WrongTanEnd::OrderDeleted,
            "HIRMG:2:2+9050::Die Nachricht enthält Fehler'\
             HIRMS:3:2:3+9941::TAN ungültig+9210::Auftrag abgelehnt - Eingereichter Auftrag gelöscht'",
        ),
        (
            WrongTanEnd::DialogEnded,
            "HIRMG:2:2+9050::Die Nachricht enthält Fehler+9800::Dialog abgebrochen'\
             HIRMS:3:2:3+9941::TAN ungültig'",
        ),
    ] {
        let bank = TestBank::new(BankModel {
            login_sca: LoginSca::Tan,
            wrong_tan: WrongTanModel {
                kept: 1,
                announce_attempts: true,
                end,
            },
            ..BankModel::default()
        })
        .handle();
        sync(&bank);
        open(&bank, true);
        let submit = |number: u32| {
            bank.exchange(&signed(
                "TESTDLG0002",
                number,
                "921",
                "TESTSYS0001",
                &["HKTAN:3:6+2++++TASK0001+N'"],
                Some("999999"),
            ))
            .expect("TAN answer")
        };
        assert_eq!(
            inner(&submit(2)),
            "HIRMG:2:2+9050::Die Nachricht enthält Fehler'\
             HIRMS:3:2:3+9941::TAN ungültig+9360::Sperrung der TAN-Liste nach weiteren 1 Fehlversuchen:1'"
        );
        assert_eq!(inner(&submit(3)), last, "{end:?}");
        bank.assert_clean();
    }
}

#[test]
fn every_answer_reads_back_with_the_codec() {
    let mut model = BankModel::default();
    model.job_mut("HKKAZ").page_size = Some(1);
    let bank = TestBank::new(model).handle();
    let iban = iban_de(BLZ, "0001234567");
    let mut answers = vec![sync(&bank), open(&bank, false)];
    let orders = [
        format!("HKSAL:3:5+0001234567::280:{BLZ}+N'"),
        format!("HKSAL:3:6+0001234567::280:{BLZ}+N'"),
        format!("HKKAZ:3:7+{iban}:TESTDEFFXXX+N+20260101+20260331'"),
        format!("HKKAZ:3:7+{iban}:TESTDEFFXXX+N+20260101+20260331++TD-1'"),
        format!("HKKAZ:3:5+0001234567::280:{BLZ}+N'"),
        format!("HKWPD:3:6+0001234567::280:{BLZ}'"),
    ];
    for (i, order) in orders.iter().enumerate() {
        answers.push(
            bank.exchange(&customer(
                "TESTDLG0002",
                2 + i as u32,
                "921",
                "TESTSYS0001",
                &[order],
            ))
            .unwrap_or_else(|v| panic!("{v}")),
        );
    }
    for answer in &answers {
        let bytes: Vec<u8> = inner(answer).chars().map(|c| c as u8).collect();
        for seg in codec::parse_message(&bytes).expect("tokenizes") {
            if let ParsedSegment::Raw { error, segment } = seg {
                panic!("{} unreadable: {error}", segment.segment_type());
            }
        }
        let outer = codec::parse_message(answer).expect("envelope tokenizes");
        assert!(outer.iter().all(|s| s.error().is_none()), "{outer:?}");
    }
    // HKWPD is not allowed for the Girokonto.
    assert!(inner(answers.last().unwrap()).contains("+9210::"));
    // The touchdown point of page 1 led to page 2.
    assert!(inner(&answers[4]).contains("3040::Es liegen weitere Informationen vor:TD-1"));
    bank.assert_clean();
}
