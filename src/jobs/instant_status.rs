//! The instant payment status job (docs/payments-design.md 3.7): HKIPS asks
//! how an HKIPZ ended up, HIIPS answers with "Status SEPA-Auftrag". A bank
//! that offers HKIPZ without HKIPS gets the statement read instead, and the
//! payment is looked up by its end-to-end ID.

use super::transfer::descriptor_for;
use super::*;
use crate::codec::decode_segment;
use crate::codec::defs::HiIps1;

/// "Status SEPA-Auftrag" (Messages 2022, data dictionary, version 2; codes
/// 1 to 5 are the version 1 list).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SepaOrderStatus {
    /// `1`: in Terminierung.
    Scheduled,
    /// `2`: abgelehnt von erster Inkassostelle.
    RejectedByFirstCollector,
    /// `3`: in Bearbeitung.
    InProcess,
    /// `4`: creditor side processed, booking initiated.
    Processed,
    /// `5`: a return (R-Transaktion) was initiated.
    ReturnInitiated,
    /// `6`: Auftrag fehlgeschlagen.
    Failed,
    /// `7`: executed; the money is available to the payee.
    Executed,
    /// `8`: rejected by the payer's payment service provider.
    RejectedByPayerPsp,
    /// `9`: rejected by the payee's payment service provider.
    RejectedByPayeePsp,
    /// A code outside the list.
    Other(String),
}

impl SepaOrderStatus {
    fn from_code(code: &str) -> Self {
        match code {
            "1" => Self::Scheduled,
            "2" => Self::RejectedByFirstCollector,
            "3" => Self::InProcess,
            "4" => Self::Processed,
            "5" => Self::ReturnInitiated,
            "6" => Self::Failed,
            "7" => Self::Executed,
            "8" => Self::RejectedByPayerPsp,
            "9" => Self::RejectedByPayeePsp,
            other => Self::Other(other.to_string()),
        }
    }
}

/// The value of [`instant_status`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstantStatus {
    /// The bank's status; `None` when the bank answered 3045 (ask again),
    /// sent no status, or the statement does not show the payment yet.
    pub status: Option<SepaOrderStatus>,
    /// With 3045: ask again not before this (arrival plus HIIPSS
    /// "Mindestwartezeit"); `None` without 3045 or without HIIPSS.
    pub again_after: Option<Timestamp>,
    /// Where `status` comes from.
    pub source: StatusSource,
}

/// Where an [`InstantStatus`] comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusSource {
    /// HIIPS, the bank's answer to HKIPS.
    StatusRequest,
    /// The account's statement from [`TransferReceipt::sent_on`] to today
    /// (HKCAZ or HKKAZ), searched for a debit with the payment's
    /// end-to-end ID: used when the bank offers no HKIPS, or the receipt
    /// has no order ID to ask with. A booked debit is
    /// [`SepaOrderStatus::Executed`], a pending one
    /// [`SepaOrderStatus::InProcess`], none found is `None`: check again
    /// later or look at the account. The client's reading, not the bank's
    /// word.
    Statement,
}

/// The status of the instant payment of `receipt` sent from `from`.
///
/// With HKIPS in the BPD and an order ID on the receipt (HIIPZ
/// "Auftragsidentifikation"), the bank is asked (HKIPS). Still open: 3045,
/// the value has `status: None` and `again_after`.
///
/// Without HKIPS, or without an order ID, the account's statement from
/// `receipt.sent_on` to today is read and searched for the payment's
/// end-to-end ID ([`StatusSource::Statement`]).
///
/// Either order follows the usual TAN rule, so the step can be `NeedTan`.
/// Fails with [`FinTSError::NoBpd`] or [`FinTSError::VersionMismatch`] before
/// anything is sent.
pub async fn instant_status(
    dialog: &mut Dialog,
    from: &Account,
    receipt: &TransferReceipt,
) -> Result<Step<InstantStatus>> {
    dialog.ready()?;
    let params = dialog.bank_params();
    let order_id = match (
        params.order_version(Order::InstantStatus),
        &receipt.order_id,
    ) {
        (Err(e @ FinTSError::NoBpd { .. }), _) => return Err(e),
        (Ok(_), Some(id)) => id.clone(),
        _ => return from_account(dialog, from, receipt).await,
    };
    let descriptor = descriptor_for(params, Order::InstantStatus)?;
    let min_wait = params
        .bpd()
        .and_then(|b| b.instant())
        .and_then(|i| i.status)
        .map(|s| s.min_wait);
    dialog
        .start(JobInput::InstantStatus {
            account: from.clone(),
            descriptors: vec![descriptor.announced],
            order_id,
            min_wait,
        })
        .await?
        .into_step()
}

/// The job's value from the bank's answer.
pub(super) fn read(response: &Response, min_wait: Option<u32>) -> Result<InstantStatus> {
    let status = response
        .find_segment("HIIPS")
        .map(|raw| {
            decode_segment::<HiIps1>(raw)
                .map(|(_, v)| v)
                .map_err(FinTSError::SegmentUnreadable)
        })
        .transpose()?
        .and_then(|s| s.status)
        .map(|c| SepaOrderStatus::from_code(c.as_str()));
    if !response.has(Kind::InstantStatusQuery) {
        return Ok(InstantStatus {
            status,
            again_after: None,
            source: StatusSource::StatusRequest,
        });
    }
    let wait = min_wait.map(|s| Duration::from_secs(u64::from(s)));
    Ok(InstantStatus {
        status: None,
        again_after: not_before_after(Timestamp::now(), wait),
        source: StatusSource::StatusRequest,
    })
}

/// The statement job that looks for the payment of `receipt`.
async fn from_account(
    dialog: &mut Dialog,
    from: &Account,
    receipt: &TransferReceipt,
) -> Result<Step<InstantStatus>> {
    info!("[FinTS] No HKIPS for this payment: reading the statement instead");
    let format = choose_format(dialog.bank_params(), from, Format::Auto)?;
    let today = dialog.inner.now().date();
    dialog
        .start(JobInput::Transactions {
            account: from.clone(),
            range: Range::new(receipt.sent_on.min(today), today),
            format,
            touchdown: None,
            booked: Vec::new(),
            pending: Vec::new(),
            pending_sent: false,
            find_payment: Some(receipt.end_to_end_id.clone()),
        })
        .await?
        .into_step()
}

/// The status of the payment with end-to-end ID `id` as `statement` shows
/// it: a booked debit is executed, a pending one in process.
pub(super) fn from_statement(statement: &Statement, id: &str) -> InstantStatus {
    let ours =
        |t: &Transaction| t.amount.is_sign_negative() && t.end_to_end_id.as_deref() == Some(id);
    let pending = match &statement.pending {
        Pending::Reported(p) => p.as_slice(),
        _ => &[],
    };
    let status = if statement.booked.iter().any(ours) {
        Some(SepaOrderStatus::Executed)
    } else if pending.iter().any(ours) {
        Some(SepaOrderStatus::InProcess)
    } else {
        None
    };
    InstantStatus {
        status,
        again_after: None,
        source: StatusSource::Statement,
    }
}
