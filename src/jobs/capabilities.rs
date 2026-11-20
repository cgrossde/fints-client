//! What a bank offers per account, from the BPD, the UPD and the versions
//! we implement, without sending anything.
//!
//! Only the BPD blocks a job (no common version, see [`Support`]). The UPD
//! allowance is shown, never enforced: the bank's answer decides.

use serde::{Deserialize, Serialize};

use super::{Format, StatementFormat, choose_statement_format, find_upd_account};
use crate::protocol::{
    Account, Allowance, BankParams, TanOverrides, UpdAccount, VersionMismatch, order_tan,
};
use crate::segments::Order;
use crate::types::{AccountKey, Iban, SegmentType};

/// The jobs [`Capabilities`] reports on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum JobKind {
    Balance,
    Transactions,
    Holdings,
    DepotTransactions,
    Transfer,
    InstantTransfer,
    InstantStatus,
}

impl JobKind {
    /// Every job, in the order of [`Capabilities`].
    pub const ALL: [JobKind; 7] = [
        JobKind::Balance,
        JobKind::Transactions,
        JobKind::Holdings,
        JobKind::DepotTransactions,
        JobKind::Transfer,
        JobKind::InstantTransfer,
        JobKind::InstantStatus,
    ];

    /// The order codes that run the job: HKKAZ and HKCAZ for transactions.
    pub(crate) fn codes(self) -> &'static [&'static str] {
        match self {
            JobKind::Transactions => &["HKKAZ", "HKCAZ"],
            JobKind::Balance => &["HKSAL"],
            JobKind::Holdings => &["HKWPD"],
            JobKind::DepotTransactions => &["HKWDU"],
            JobKind::Transfer => &["HKCCS"],
            JobKind::InstantTransfer => &["HKIPZ"],
            JobKind::InstantStatus => &["HKIPS"],
        }
    }

    fn order(self) -> Order {
        match self {
            JobKind::Balance => Order::Balance,
            JobKind::Transactions => Order::Transactions,
            JobKind::Holdings => Order::Holdings,
            JobKind::DepotTransactions => Order::DepotTransactions,
            JobKind::Transfer => Order::Transfer,
            JobKind::InstantTransfer => Order::InstantTransfer,
            JobKind::InstantStatus => Order::InstantStatus,
        }
    }
}

/// Whether the BPD lets us send a job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub enum Support {
    /// The job would be sent as `code` in `version`. For
    /// [`JobKind::Transactions`] `code` is `HKCAZ` or `HKKAZ`, whichever
    /// [`transactions`](super::transactions) with `Format::Auto` would
    /// use, and `formats` lists the camt formats the bank offers (empty
    /// for MT940 and every other job).
    Available {
        code: &'static str,
        version: u16,
        formats: Vec<String>,
        /// The bank's storage period in days (Speicherzeitraum of
        /// HIKAZS/HICAZS) for [`JobKind::Transactions`]; `None` for every
        /// other job or when the BPD gives none. A statement range that
        /// starts earlier is clamped by [`transactions`](super::transactions).
        days_stored: Option<u32>,
    },
    /// The BPD has no parameter segment for the job.
    NotOffered,
    /// The BPD offers versions, none of which we implement.
    VersionMismatch {
        offered: Vec<u16>,
        implemented: Vec<u16>,
    },
    /// No BPD is known (no dialog has run, nothing cached).
    NoBpd,
}

/// One job on one account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JobCapability {
    pub job: JobKind,
    pub support: Support,
    /// What the UPD says about the order in `support` (for transactions,
    /// the order that would be used). Information only: `No` does not stop
    /// the job. `Unknown` also when the UPD is not known, see
    /// [`SessionState::capabilities`](crate::SessionState::capabilities).
    pub upd: Allowance,
    /// Whether the order needs a TAN: HIPINS, learned codes and the
    /// caller's overrides. `None` when nothing says.
    pub tan_required: Option<bool>,
}

/// The jobs of one account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AccountCapabilities {
    /// The UPD entry's key when the UPD names the account (the same as
    /// [`AccountInfo::key`](super::AccountInfo::key)), else the key the
    /// account's own number and BLZ, or IBAN, give.
    pub key: AccountKey,
    /// Whether the UPD names the account.
    pub in_upd: bool,
    /// One entry per [`JobKind::ALL`].
    pub jobs: Vec<JobCapability>,
    /// The payee check (VoP): `None` when the BPD has no HIVPPS.
    pub payee_check: Option<VopSupport>,
}

/// What the BPD says about the payee check (HIVPPS).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VopSupport {
    /// "VOP-pflichtige Zahlungsverkehrsaufträge": the orders that go with
    /// an HKVPP.
    pub required_for: Vec<SegmentType>,
}

impl AccountCapabilities {
    /// The entry for `job`.
    pub fn job(&self, job: JobKind) -> Option<&JobCapability> {
        self.jobs.iter().find(|j| j.job == job)
    }
}

/// What the bank offers, per account.
///
/// Built without a dialog from a stored [`SessionState`](crate::SessionState),
/// or from an open [`Dialog`](super::Dialog). A bank that sends the UPD for
/// one dialog only (version 0) leaves no UPD in the session state: from
/// there the accounts must be named by the caller and every allowance is
/// `Unknown`; the dialog knows the UPD.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Capabilities {
    pub accounts: Vec<AccountCapabilities>,
}

impl Capabilities {
    /// Reports `accounts`, or the UPD's accounts when `accounts` is empty.
    pub(crate) fn build(
        params: &BankParams,
        overrides: &TanOverrides,
        accounts: &[Account],
    ) -> Capabilities {
        let accounts = if accounts.is_empty() {
            params
                .upd()
                .map(|u| u.accounts().iter().collect::<Vec<_>>())
                .unwrap_or_default()
                .into_iter()
                .filter_map(|a| Some(account_capabilities(params, overrides, a.key()?, Some(a))))
                .collect()
        } else {
            accounts
                .iter()
                .map(|a| {
                    let upd = find_upd_account(params, a);
                    let key = upd.and_then(UpdAccount::key).unwrap_or_else(|| key_of(a));
                    account_capabilities(params, overrides, key, upd)
                })
                .collect()
        };
        Capabilities { accounts }
    }
}

/// The key of an account the UPD does not name: number, subaccount and
/// BLZ, else IBAN and subaccount.
fn key_of(a: &Account) -> AccountKey {
    let sub = a.subaccount();
    a.blz()
        .zip(a.account_number())
        .and_then(|(blz, number)| AccountKey::national(blz, number, sub))
        .or_else(|| AccountKey::iban(a.iban(), sub))
        .unwrap_or_else(|| AccountKey::Iban {
            iban: Iban::new(a.iban()),
            subaccount: None,
        })
}

fn account_capabilities(
    params: &BankParams,
    overrides: &TanOverrides,
    key: AccountKey,
    upd: Option<&UpdAccount>,
) -> AccountCapabilities {
    let allowance = |code: &str| match (params.upd(), upd) {
        (Some(u), Some(a)) => u.allows(a, code),
        _ => Allowance::Unknown,
    };
    let jobs = JobKind::ALL
        .iter()
        .map(|&job| {
            let mut order = job.order();
            if job == JobKind::Transactions
                && matches!(
                    choose_statement_format(params, Format::Auto, || allowance("HKCAZ")),
                    Ok(StatementFormat::Camt)
                )
            {
                order = Order::Camt;
            }
            let support = match params.order_version(order) {
                Ok(version) => Support::Available {
                    code: order.code(),
                    version,
                    formats: if order == Order::Camt {
                        params
                            .bpd()
                            .map(|b| b.camt_formats().to_vec())
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    },
                    days_stored: (job == JobKind::Transactions)
                        .then(|| params.days_stored(order))
                        .flatten(),
                },
                Err(crate::FinTSError::VersionMismatch(VersionMismatch {
                    offered,
                    implemented,
                    ..
                })) => {
                    if offered.is_empty() {
                        Support::NotOffered
                    } else {
                        Support::VersionMismatch {
                            offered,
                            implemented,
                        }
                    }
                }
                Err(_) => Support::NoBpd,
            };
            let code = order.code();
            let tan = order_tan(
                &SegmentType::new(code),
                true,
                params.bpd().and_then(|b| b.tan_required(code)),
                overrides,
                &params.tan_required_learned,
            );
            JobCapability {
                job,
                support,
                upd: allowance(code),
                tan_required: match tan.reason {
                    crate::protocol::TanReason::Unlisted => None,
                    _ => Some(tan.hktan),
                },
            }
        })
        .collect();
    AccountCapabilities {
        key,
        in_upd: upd.is_some(),
        jobs,
        payee_check: params.bpd().and_then(|b| b.vop()).map(|v| VopSupport {
            required_for: v.required_for.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::wire::tokenize;
    use crate::protocol::{TanOverride, TanOverrides};
    use crate::session::SessionState;
    use crate::types::{Blz, SystemId, UserId};

    const IBAN: &str = "DE00123456781234567890";

    fn params(bpd: &str, upd: &str) -> BankParams {
        let mut p = BankParams::new();
        if !bpd.is_empty() {
            let wire = format!("HIBPA:1:3:4+1+280:1+Bank+1+1+300'{bpd}");
            p.set_bpd(tokenize(wire.as_bytes()).unwrap());
        }
        if !upd.is_empty() {
            p.set_upd(tokenize(upd.as_bytes()).unwrap());
        }
        p
    }

    fn upd(version: u8, usage: u8, allowed: &str) -> String {
        format!(
            "HIUPA:5:4:4+USER0001+{version}+{usage}'HIUPD:6:6:4+1234567890::280:12345678+{IBAN}+CUST0001+1+EUR+Erika Beispiel++Girokonto++{allowed}'"
        )
    }

    fn account() -> Account {
        Account::new(IBAN, "TESTDEFFXXX").unwrap()
    }

    fn caps(p: &BankParams, accounts: &[Account]) -> Capabilities {
        Capabilities::build(p, &TanOverrides::default(), accounts)
    }

    fn of(c: &Capabilities, job: JobKind) -> &JobCapability {
        c.accounts[0].job(job).unwrap()
    }

    const BPD: &str = "HISALS:2:6:4+1+1+0'HIKAZS:3:7:4+1+1+0'HICAZS:4:1:4+1+1+0+450:N:N:urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.02'HIWPDS:5:9:4+1+1+0'";

    #[test]
    fn closed_upd_steers_to_mt940_and_shows_no() {
        let p = params(BPD, &upd(5, 0, "HKSAL:1+HKKAZ:1"));
        let c = caps(&p, &[]);
        assert_eq!(c.accounts.len(), 1);
        assert!(c.accounts[0].in_upd);
        let bal = of(&c, JobKind::Balance);
        assert_eq!(bal.upd, Allowance::Yes);
        assert_eq!(
            bal.support,
            Support::Available {
                code: "HKSAL",
                version: 6,
                formats: vec![],
                days_stored: None
            }
        );
        // HKCAZ is offered but not listed in a closed UPD: MT940.
        let tx = of(&c, JobKind::Transactions);
        assert_eq!(tx.upd, Allowance::Yes);
        assert!(matches!(
            &tx.support,
            Support::Available {
                code: "HKKAZ",
                version: 7,
                ..
            }
        ));
        // Version 9 only: we do not implement it. UPD `No` is shown.
        let h = of(&c, JobKind::Holdings);
        assert_eq!(h.upd, Allowance::No);
        assert_eq!(
            h.support,
            Support::VersionMismatch {
                offered: vec![9],
                implemented: Order::Holdings.implemented().to_vec()
            }
        );
        assert_eq!(of(&c, JobKind::InstantStatus).support, Support::NotOffered);
    }

    #[test]
    fn open_upd_prefers_camt_with_its_formats() {
        let p = params(BPD, &upd(5, 1, "HKSAL:1"));
        let c = caps(&p, &[]);
        let tx = of(&c, JobKind::Transactions);
        assert_eq!(tx.upd, Allowance::Unknown);
        let Support::Available {
            code,
            version,
            formats,
            days_stored,
        } = &tx.support
        else {
            panic!("{tx:?}")
        };
        assert_eq!((*code, *version), ("HKCAZ", 1));
        assert_eq!(*days_stored, Some(450));
        assert_eq!(formats.len(), 1);
        assert!(formats[0].contains("camt.052"));
    }

    #[test]
    fn named_accounts_unknown_to_the_upd_are_unknown() {
        let p = params(BPD, &upd(5, 0, "HKSAL:1"));
        let other = Account::new("DE00123456780000000001", "TESTDEFFXXX").unwrap();
        let c = caps(&p, &[other]);
        assert!(!c.accounts[0].in_upd);
        assert_eq!(of(&c, JobKind::Balance).upd, Allowance::Unknown);
    }

    #[test]
    fn dialog_only_upd_is_in_the_dialog_not_the_session() {
        let p = params(BPD, &upd(0, 0, "HKSAL:1"));
        // The dialog knows the UPD.
        assert_eq!(of(&caps(&p, &[]), JobKind::Holdings).upd, Allowance::No);
        // The stored session state does not: no accounts of its own, and
        // `Unknown` for the ones the caller names.
        let state = SessionState::capture(
            &Blz::new("12345678"),
            &UserId::new("USER0001"),
            &SystemId::new("0"),
            &p,
        );
        assert!(state.capabilities(&[]).accounts.is_empty());
        let c = state.capabilities(&[account()]);
        assert!(!c.accounts[0].in_upd);
        assert_eq!(of(&c, JobKind::Holdings).upd, Allowance::Unknown);
        assert!(matches!(
            of(&c, JobKind::Balance).support,
            Support::Available { version: 6, .. }
        ));
    }

    #[test]
    fn stored_upd_gives_the_same_answer_as_the_dialog() {
        let p = params(BPD, &upd(5, 0, "HKSAL:1"));
        let state = SessionState::capture(
            &Blz::new("12345678"),
            &UserId::new("USER0001"),
            &SystemId::new("0"),
            &p,
        );
        assert_eq!(state.capabilities(&[]), caps(&p, &[]));
    }

    #[test]
    fn no_bpd_refuses_everything_and_shows_the_upd() {
        let p = params("", &upd(5, 0, "HKSAL:1"));
        let c = caps(&p, &[]);
        for j in &c.accounts[0].jobs {
            assert_eq!(j.support, Support::NoBpd, "{:?}", j.job);
        }
        assert_eq!(of(&c, JobKind::Balance).upd, Allowance::Yes);
        // No BPD and no UPD: nothing to report unless the caller names accounts.
        let p = params("", "");
        assert!(caps(&p, &[]).accounts.is_empty());
        assert_eq!(
            caps(&p, &[account()]).accounts[0].jobs.len(),
            JobKind::ALL.len()
        );
    }

    #[test]
    fn tan_requirement_from_learning_and_overrides() {
        let mut p = params(BPD, &upd(5, 1, "HKSAL:1"));
        assert_eq!(of(&caps(&p, &[]), JobKind::Balance).tan_required, None);
        p.tan_required_learned.insert(SegmentType::new("HKSAL"));
        assert_eq!(
            of(&caps(&p, &[]), JobKind::Balance).tan_required,
            Some(true)
        );
        let overrides: TanOverrides = [(SegmentType::new("HKSAL"), TanOverride::Never)]
            .into_iter()
            .collect();
        let c = Capabilities::build(&p, &overrides, &[]);
        assert_eq!(of(&c, JobKind::Balance).tan_required, Some(false));
    }
}
