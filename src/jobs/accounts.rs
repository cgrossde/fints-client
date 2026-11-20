//! The accounts of the UPD as the caller sees them after the login.

use serde::{Deserialize, Serialize};
use tracing::warn;

use super::JobKind;
use crate::error::{FinTSError, Result};
use crate::protocol::{Account, AccountLimit, Upd, UpdAccount};
use crate::types::{AccountClass, AccountKey, Bic, ClassSource, Currency, Iban, classify};

/// One account of the UPD (HIUPD), ready to run jobs on with
/// [`AccountInfo::account`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountInfo {
    /// The account's identity, stable across logins (see [`AccountKey`]).
    pub key: AccountKey,
    /// `None` when the UPD gives none (a depot or card at some banks).
    pub iban: Option<Iban>,
    /// The UPD's BIC when it has one, else the bank's.
    pub bic: Bic,
    /// Kontonummer, as the bank sent it.
    pub number: Option<String>,
    /// Unterkontomerkmal, as the bank sent it.
    pub subaccount: Option<String>,
    /// Name des Kontoinhabers 1 and 2, those given.
    pub owner: Vec<String>,
    pub product_name: Option<String>,
    pub currency: Option<Currency>,
    /// Kontoart (HIUPD 6) as the bank sent it; see [`AccountInfo::class`].
    pub account_type: Option<u8>,
    /// What kind of account this is, and where the answer comes from:
    ///
    /// 1. The Kontoart, by the ranges of the FinTS Formals data dictionary
    ///    ("Kontoart"): [`ClassSource::Kontoart`].
    /// 2. With no Kontoart (or one outside 1-99): a depot when the UPD
    ///    allows HKWPD and none of HKSAL, HKKAZ, HKCAZ:
    ///    [`ClassSource::AllowedJobs`].
    /// 3. Otherwise `None`: the library does not guess a checking account.
    ///
    /// `product_name` and `account_type` stay as the bank sent them, so a
    /// caller can reclassify by what only it knows.
    pub class: Option<(AccountClass, ClassSource)>,
    /// Kontolimit.
    pub limit: Option<AccountLimit>,
    /// The jobs whose order the UPD lists for the account (Erlaubte
    /// Geschäftsvorfälle), in the order of [`JobKind::ALL`]; transactions
    /// when it lists HKKAZ or HKCAZ. Empty when it lists none.
    pub allowed: Vec<JobKind>,
}

impl AccountInfo {
    /// The entries of `upd`, with `bank_bic` where the UPD has no BIC. An
    /// entry with neither account number nor IBAN is left out (logged).
    pub(crate) fn from_upd(upd: &Upd, bank_bic: &Bic) -> Vec<AccountInfo> {
        upd.accounts()
            .iter()
            .filter_map(|a| {
                let info = Self::from_upd_account(a, bank_bic);
                if info.is_none() {
                    warn!("[FinTS] A UPD account has neither number nor IBAN; left out");
                }
                info
            })
            .collect()
    }

    fn from_upd_account(a: &UpdAccount, bank_bic: &Bic) -> Option<AccountInfo> {
        let lists = |code: &str| a.allowed.keys().any(|c| c.as_str() == code);
        Some(AccountInfo {
            key: a.key()?,
            iban: a.iban.clone(),
            bic: a.bic.clone().unwrap_or_else(|| bank_bic.clone()),
            number: a.account_number.clone(),
            subaccount: a.sub_account.clone(),
            owner: a.owner.clone(),
            product_name: a.product_name.clone(),
            currency: a.currency.clone(),
            account_type: a.account_type,
            class: classify(a.account_type, lists),
            limit: a.limit.clone(),
            allowed: JobKind::ALL
                .into_iter()
                .filter(|j| j.codes().iter().any(|c| lists(c)))
                .collect(),
        })
    }

    /// May the user run `job` on this account, by the UPD? `true` when the
    /// UPD lists no orders at all (nothing is known), else only for a
    /// listed one. Information only: the jobs never check it.
    pub fn allows(&self, job: JobKind) -> bool {
        self.allowed.is_empty() || self.allowed.contains(&job)
    }

    /// Whether this is a securities depot by [`AccountInfo::class`].
    pub fn is_depot(&self) -> bool {
        matches!(self.class, Some((AccountClass::Depot, _)))
    }

    /// The account to run jobs on: by IBAN with the national parts of the
    /// UPD, or nationally (number, subaccount, BLZ) without an IBAN.
    /// Fails when the BIC is empty, or for an account without IBAN whose
    /// key has no BLZ.
    pub fn account(&self) -> Result<Account> {
        let bic = self.bic.as_str();
        match (&self.iban, &self.key) {
            (Some(iban), AccountKey::National { blz, .. }) => {
                let number = self.number.as_deref().unwrap_or_default();
                Ok(Account::new(iban.as_str(), bic)?.with_national(
                    number,
                    self.subaccount.as_deref(),
                    blz,
                ))
            }
            (Some(iban), AccountKey::Iban { .. }) => {
                Ok(Account::new(iban.as_str(), bic)?.with_subaccount(self.subaccount.as_deref()))
            }
            (None, AccountKey::National { blz, .. }) => Account::national(
                self.number.as_deref().unwrap_or_default(),
                self.subaccount.as_deref(),
                blz,
                bic,
            ),
            (None, AccountKey::Iban { .. }) => Err(FinTSError::Dialog(
                "the account has neither IBAN nor bank code".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::wire::tokenize;

    fn infos(hiupd: &str) -> Vec<AccountInfo> {
        let upd = Upd::from_segments(
            &tokenize(format!("HIUPA:5:4:4+USER0001+1+0'{hiupd}").as_bytes()).unwrap(),
        )
        .unwrap();
        AccountInfo::from_upd(&upd, &Bic::new("TESTDEFFXXX"))
    }

    /// A clearing account and its depot under one IBAN and number, told
    /// apart by the subaccount; a card without IBAN; an entry by IBAN only.
    #[test]
    fn upd_entries_become_account_infos() {
        let a = infos(
            "HIUPD:6:6:4+0000000301::280:12345678+DE00123456780000000301+CUST0001+1+EUR+Erika Beispiel++Girokonto+T:1000,:EUR+HKSAL:1+HKCAZ:1'\
             HIUPD:7:6:4+0000000301:01:280:12345678+DE00123456780000000301+CUST0001++EUR+Erika Beispiel++Depot++HKWPD:1'\
             HIUPD:8:6:4+4111000000000001:CARD01:280:12345678++CUST0001+50+EUR+Erika Beispiel++Karte++DKKKU:1'\
             HIUPD:9:6:4++DE00123456780000000999+CUST0001+10+EUR+Erika Beispiel++Tagesgeld++HKSAL:1'",
        );
        assert_eq!(a.len(), 4);
        assert_eq!(a[0].key.to_string(), "12345678/301");
        assert_eq!(
            a[0].class,
            Some((AccountClass::Checking, ClassSource::Kontoart))
        );
        assert_eq!(a[0].allowed, [JobKind::Balance, JobKind::Transactions]);
        assert!(a[0].limit.is_some());
        assert_eq!(a[0].bic.as_str(), "TESTDEFFXXX");
        assert_eq!(a[1].key.to_string(), "12345678/301/01");
        assert_ne!(a[0].key, a[1].key);
        assert!(a[1].is_depot());
        assert!(!a[1].allows(JobKind::Balance));
        assert_eq!(a[2].key.to_string(), "12345678/4111000000000001/CARD01");
        assert!(a[2].allowed.is_empty(), "an order this client does not run");
        assert_eq!(a[3].key.to_string(), "iban:DE00123456780000000999");

        let clearing = a[0].account().unwrap();
        assert_eq!(clearing.iban(), "DE00123456780000000301");
        assert_eq!(clearing.subaccount(), None);
        let depot = a[1].account().unwrap();
        assert_eq!(depot.subaccount(), Some("01"));
        let card = a[2].account().unwrap();
        assert_eq!(card.iban(), "");
        assert_eq!(card.account_number(), Some("4111000000000001"));
        assert_eq!(a[3].account().unwrap().iban(), "DE00123456780000000999");
    }

    /// Each entry's account finds its own entry again, also when they share
    /// IBAN and number; an account built by hand from the shared IBAN finds
    /// the entry without subaccount.
    #[test]
    fn accounts_find_their_own_upd_entry() {
        let hiupd = "HIUPD:6:6:4+0000000301::280:12345678+DE00123456780000000301+CUST0001+1+EUR+Erika Beispiel++Girokonto++HKSAL:1+HKCAZ:1'\
             HIUPD:7:6:4+0000000301:01:280:12345678+DE00123456780000000301+CUST0001++EUR+Erika Beispiel++Depot++HKWPD:1'\
             HIUPD:9:6:4++DE00123456780000000999+CUST0001+10+EUR+Erika Beispiel++Tagesgeld++HKSAL:1'";
        let upd = Upd::from_segments(
            &tokenize(format!("HIUPA:5:4:4+USER0001+1+0'{hiupd}").as_bytes()).unwrap(),
        )
        .unwrap();
        for info in AccountInfo::from_upd(&upd, &Bic::new("TESTDEFFXXX")) {
            let found = upd.find(&info.account().unwrap()).expect("found");
            assert_eq!(found.key().as_ref(), Some(&info.key));
        }
        let by_hand = Account::new("DE00123456780000000301", "TESTDEFFXXX").unwrap();
        assert_eq!(
            upd.find(&by_hand).and_then(UpdAccount::key),
            infos(hiupd).first().map(|i| i.key.clone())
        );
    }
}
