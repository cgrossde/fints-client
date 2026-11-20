//! The shipped profiles and the bank codes each one covers.
//!
//! An entry needs a trace or a docs/banks.md observation where the generic
//! rule gives the wrong result; a TAN override also needs the maintainer's
//! agreement. BLZ lists are explicit: one URL can serve several banks, so a
//! URL is not an identity. A test checks that every directory code sharing
//! a profiled bank's URL is listed.

use super::profile::{BankProfile, BookingIdSource, InitMode, ProfileId, UserIdRule};

/// Bank codes and the constructor of their profile.
pub(super) type Entry = (&'static [&'static str], fn() -> BankProfile);

/// Bank codes → profile constructor.
pub(super) const PROFILES: &[Entry] = &[
    (&["12030000"], dkb),
    (
        &[
            "20041111", "20041133", "20041144", "20041155", "20041166", "20041177", "20041188",
            "20041199",
        ],
        comdirect,
    ),
    // 70120400: the former DAB code, served by the same host.
    (&["76030080", "70120400"], consorsbank),
    (&["50010517"], ing),
];

/// DKB (docs/banks/dkb.md): login, HKTAN per order and HKCAZ follow the
/// BPD; the BIC comes from the directory. The camt entry reference
/// (`Ntry/AcctSvcrRef`) is unique per booking (live run, November 2026), so
/// booked entries get it as their booking ID; pending ones do not, since
/// whether a pending booking keeps its reference is unverified.
fn dkb() -> BankProfile {
    BankProfile {
        id: ProfileId::Dkb,
        booking_id: BookingIdSource::CamtEntryAcctSvcrRef,
        ..BankProfile::generic()
    }
}

/// comdirect, all directory codes (docs/banks/comdirect.md): job versions
/// and the depot rule are generic. No behaviour fields.
fn comdirect() -> BankProfile {
    BankProfile {
        id: ProfileId::Comdirect,
        ..BankProfile::generic()
    }
}

/// Consorsbank (docs/banks/consorsbank.md): no TAN override, `9075`
/// learning covers HIPINS. The FinTS user ID is
/// the account number (9 digits, 10 for former DAB customers) plus the user
/// number `001`; the library completes the short form (the source of
/// that rule is unverified).
fn consorsbank() -> BankProfile {
    BankProfile {
        id: ProfileId::Consorsbank,
        user_id: Some(UserIdRule {
            min_digits: 9,
            max_digits: 10,
            suffix: "001".into(),
        }),
        ..BankProfile::generic()
    }
}

/// ING (docs/banks/ing.md, **unverified**): business dialog without a
/// synchronisation dialog, security function 900.
fn ing() -> BankProfile {
    BankProfile {
        id: ProfileId::Ing,
        init: InitMode::Direct {
            security_function: "900".into(),
        },
        ..BankProfile::generic()
    }
}

/// The profile for `blz`, generic when no entry lists it.
pub(super) fn profile_for(blz: &str) -> BankProfile {
    PROFILES
        .iter()
        .find(|(codes, _)| codes.contains(&blz))
        .map_or_else(BankProfile::generic, |(_, make)| make())
}
