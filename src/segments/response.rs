//! Readers for the two segments that can arrive in a version without a
//! declarative definition in `codec::defs`. `segments::read` calls them
//! only when no definition exists:
//!
//! - HITANS above version 7. No published table defines one, so it is read
//!   with the version 7 layout; methods of a version this client cannot
//!   send are never selected.
//! - HIUPD in versions other than 4 and 6. Version 5 is shown only by
//!   example (Formals 2017); versions 1 to 3 are HBCI 2.0.1 and 2.1, which
//!   a FinTS 3.0 bank does not speak.
//!
//! Both read positions of the tokenized segment (`codec::wire`) and accept
//! whatever is there.

use crate::parser::{DEG, RawSegment};
use crate::types::*;

/// Whether `text` has the shape of an IBAN: country letters, two check
/// digits, then letters and digits, 15 to 34 characters in all. The check
/// digits are not verified.
fn looks_like_iban(text: &str) -> bool {
    let Some((head, rest)) = text.as_bytes().split_first_chunk::<4>() else {
        return false;
    };
    (15..=34).contains(&text.len())
        && head[..2].iter().all(u8::is_ascii_uppercase)
        && head[2..].iter().all(u8::is_ascii_digit)
        && rest.iter().all(u8::is_ascii_alphanumeric)
}

/// Whether `text` has the shape of a BIC: six letters (bank and country),
/// then a two-character location and an optional three-character branch.
fn looks_like_bic(text: &str) -> bool {
    let bytes = text.as_bytes();
    matches!(bytes.len(), 8 | 11)
        && bytes[..6].iter().all(u8::is_ascii_alphabetic)
        && bytes[6..].iter().all(u8::is_ascii_alphanumeric)
}

/// Elements before the first method in the HITANS parameter group: one-step
/// allowed, several TAN methods per order, order hash.
const METHODS_START: usize = 3;
/// Elements per method in TAN2StepParams7 (PIN/TAN 2020, D).
const METHOD_LEN: usize = 26;

/// Positions inside one TAN2StepParams7 method.
mod at {
    pub(super) const SECURITY_FUNCTION: usize = 0;
    pub(super) const TAN_PROCESS: usize = 1;
    pub(super) const DK_NAME: usize = 3;
    pub(super) const NAME: usize = 5;
    /// 0 no, 1 optional, 2 required.
    pub(super) const MEDIUM_REQUIRED: usize = 18;
    pub(super) const MAX_POLLS: usize = 21;
    pub(super) const WAIT_FIRST: usize = 22;
    pub(super) const WAIT_NEXT: usize = 23;
    pub(super) const AUTO_POLLING: usize = 25;
}

/// The TAN methods of a HITANS above version 7, read as TAN2StepParams7.
///
/// All methods sit in one group, DE 5 (index 4): `+J:N:0:<method>:<method>`.
/// A method with an empty security function is skipped.
pub(crate) fn parse_hitans(seg: &RawSegment) -> Vec<TanMethod> {
    let group = seg.deg(4);
    (METHODS_START..group.len())
        .step_by(METHOD_LEN)
        .filter_map(|start| tan_method(group, start, seg.segment_version()))
        .collect()
}

fn tan_method(group: &DEG, start: usize, version: u16) -> Option<TanMethod> {
    let element = |offset: usize| group.get_str(start + offset);
    let number = |offset: usize| element(offset).parse::<i32>().ok();

    let security_function = element(at::SECURITY_FUNCTION);
    if security_function.is_empty() {
        return None;
    }
    let dk_name = Some(element(at::DK_NAME)).filter(|n| !n.is_empty());
    let name = Some(element(at::NAME))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| format!("TAN-{security_function}"));
    let polls = [at::MAX_POLLS, at::WAIT_FIRST, at::WAIT_NEXT].map(number);
    // The poll parameters are filled for decoupled methods only; the DK
    // names of those begin with "Decoupled".
    let decoupled_name = dk_name
        .as_deref()
        .is_some_and(|n| n.to_ascii_lowercase().starts_with("decouple"));
    let [max_polls, wait_first, wait_next] = polls;

    Some(TanMethod {
        name,
        tan_process: TanProcess::from_str_val(&element(at::TAN_PROCESS)),
        needs_tan_medium: element(at::MEDIUM_REQUIRED) == "2",
        decoupled_max_polls: max_polls.unwrap_or(-1),
        wait_before_first_poll: wait_first.unwrap_or_default(),
        wait_before_next_poll: wait_next.unwrap_or_default(),
        is_decoupled: decoupled_name || polls.iter().any(Option::is_some),
        hktan_version: version,
        decoupled_auto_polling: match element(at::AUTO_POLLING).as_str() {
            "J" => Some(true),
            "N" => Some(false),
            _ => None,
        },
        challenge_structured: false,
        security_function: SecurityFunction::new(security_function),
        zka_name: dk_name,
    })
}

/// The account of a HIUPD without a definition.
///
/// Positions follow version 6 (KInfo6): 1 KTV, 2 IBAN, 3 customer ID,
/// 4 Kontoart, 5 currency, 6 and 7 owner, 8 product, 9 limit, then the
/// allowed orders. Other versions put the IBAN elsewhere, so an IBAN not
/// found at position 2 of a version 6 or later is searched in every group;
/// a BIC likewise, from position 2 on. The Kontoart is trusted in version
/// 6 or later only.
pub(crate) fn parse_hiupd(seg: &RawSegment) -> Option<SepaAccount> {
    if seg.deg_count() < 3 {
        return None;
    }
    let version = seg.segment_version();
    let first_of_group = |index: usize| seg.deg(index).get_str(0);
    let text = |index: usize| read_opt_str(seg, index, 0);
    let ktv = seg.deg(1);

    let iban = Some(first_of_group(2))
        .filter(|candidate| version >= 6 && looks_like_iban(candidate))
        .or_else(|| {
            (1..seg.deg_count())
                .map(first_of_group)
                .find(|c| looks_like_iban(c))
        })
        .unwrap_or_default();
    let bic = (2..seg.deg_count())
        .map(first_of_group)
        .find(|c| looks_like_bic(c))
        .unwrap_or_default();
    let account_type = text(4)
        .filter(|_| version >= 6)
        .and_then(|kind| kind.parse::<u8>().ok())
        .filter(|kind| (1..=99).contains(kind));

    Some(SepaAccount {
        iban: Iban::new(iban),
        bic: Bic::new(bic),
        account_number: ktv.get_str(0),
        sub_account: ktv.get_str(1),
        blz: Blz::new(ktv.get_str(3)),
        owner: text(6).or_else(|| text(7)),
        product_name: text(8).or_else(|| text(9)),
        currency: text(5).map(Currency::new),
        account_type,
        allowed_jobs: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(wire: &str) -> RawSegment {
        let mut segments = crate::codec::wire::tokenize(wire.as_bytes()).unwrap();
        segments.swap_remove(0)
    }

    #[test]
    fn iban_shape() {
        for iban in [
            "DE00123456780000004711",
            "NL00TEST0123456789",
            "MT00TEST12345ABCDEFGHIJKLMNOPQR",
        ] {
            assert!(looks_like_iban(iban), "{iban}");
        }
        for not_iban in [
            "",
            "DE00",
            "DE0012345678",
            "0012345678000000471100",
            "de00123456780000004711",
            "DE0A123456780000004711",
            "DE00-12345678-00000047",
        ] {
            assert!(!looks_like_iban(not_iban), "{not_iban}");
        }
    }

    #[test]
    fn bic_shape() {
        for bic in ["TESTDEFF", "TESTDEFFXXX", "TESTDE33A01"] {
            assert!(looks_like_bic(bic), "{bic}");
        }
        for not_bic in ["", "TESTDE", "TESTDEFF1", "TEST1EFFXXX", "TESTDEFF-XX"] {
            assert!(!looks_like_bic(not_bic), "{not_bic}");
        }
    }

    #[test]
    fn hitans_8_is_read_as_version_7() {
        let seg = segment(
            "HITANS:9:8:4+1+1+1+N:N:0:\
             :2:LEER:HHD::Leer:::TAN:6:N:1:N:0:0:N:N:00:0:N:1::::::\
             944:2:TAPP:DecoupledPush:::::Freigabe:2048:N:1:N:0:0:N:N:00:2:N:1:90:5:2:J:N'",
        );
        let methods = parse_hitans(&seg);
        let [app] = methods.as_slice() else {
            panic!("{methods:?}");
        };
        assert_eq!(app.security_function.as_str(), "944");
        assert_eq!(app.name, "TAN-944", "no name given");
        assert_eq!(app.hktan_version, 8);
        assert!(app.is_decoupled && app.needs_tan_medium);
        assert_eq!(
            (
                app.decoupled_max_polls,
                app.wait_before_first_poll,
                app.wait_before_next_poll
            ),
            (90, 5, 2)
        );
        assert_eq!(app.decoupled_auto_polling, Some(false));
    }

    /// HIUPD 5 places the IBAN last; it is found by its shape, and the
    /// Kontoart position is not read.
    #[test]
    fn hiupd_5_finds_the_iban_by_shape() {
        let seg = segment(
            "HIUPD:5:5:4+4711::280:12345678+kunde7+EUR+Erika Muster++Girokonto+DE00123456780000004711+TESTDEFFXXX'",
        );
        let account = parse_hiupd(&seg).unwrap();
        assert_eq!(account.iban.as_str(), "DE00123456780000004711");
        assert_eq!(account.bic.as_str(), "TESTDEFFXXX");
        assert_eq!(
            (account.account_number.as_str(), account.blz.as_str()),
            ("4711", "12345678")
        );
        assert_eq!(account.account_type, None);
        assert!(parse_hiupd(&segment("HIUPD:5:5:4+4711'")).is_none());
    }
}
