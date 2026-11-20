# Return codes: error and status types

Spec basis: Formals 2017 B.7.2 (HIRMG), B.7.3 (HIRMS), B.7.5.1-2 (classes, reaction rules); Rückmeldungscodes 2026-02-03 (`spec/FinTS_Rueckmeldungscodes_2026-02-03_FV.txt`, "RM" below) B.1-B.4, C.6 (3072), C.7 (9000). Bank behaviour: [banks.md](banks.md). Steps and dialogs: [jobs-step-design.md](jobs-step-design.md).

## 1. Classification

`src/return_code.rs`: `Kind::of(code, params)` gives the meaning of a code, `Category::of(code, &kind)` the reaction to a class 9 code, `FinTSError::kind()` the category of every error. A code `Kind::of` does not list is `Kind::Other`. The raw code, text, scope, reference element and parameters stay on the `ReturnCode`, so an unknown code loses nothing.

| Code | Meaning (RM) | `Kind` | Handling |
|---|---|---|---|
| 0010, 0020, 0100, 0900 | received / executed / dialog ended / TAN valid | `MessageReceived`, `OrderExecuted`, `DialogEnded`, `TanValid` | success; `OrderExecuted` marks a final answer |
| 0025 | payee check without name deviation | `NoNameDeviation` | success |
| 0030 | order received, TAN required | `TanRequired` | consumed: TAN step |
| 3010 | not available (RM: no new entries, information not offered, data up to date) | `NotAvailable` | notice; a decoupled confirmation without data segment counts as complete; `Response::has_no_entries()` |
| 3020 | BIC outdated, P1 new BIC | `BicOutdated` | notice |
| 3040 | more data, P1 touchdown point | `Touchdown` | consumed: paging |
| 3045 | instant payment status unknown yet | `InstantStatusQuery` | consumed |
| 3046 | check the account's transactions | `CheckTransactions` | notice |
| 3060 | partial warnings | `PartialWarnings` | notice |
| 3070 | order accepted, P1 bank reference | `ProcessingReference` | notice; the transfer job reads it (`bank_reference`) |
| 3072 | new access data, P1 user ID, P2 customer ID | `UserDataChanged` | notice; also `changed_user_id` in `SessionState`, UPD dropped |
| 3076 | no SCA required | `ScaExemption` | consumed: no TAN challenge |
| 3090, 3093, 3094 | payee check result / in progress / complete | `VopCheckResult`, `VopInProgress`, `VopComplete` | consumed in the answer to HKVPP or HKVPA (`Scope::Segment { order }`); notice elsewhere |
| 3091, 3945 | VoP not required / approval not possible | `VopNotRequired`, `VopApprovalNotPossible` | consumed; debug log |
| 3092 | VoP required from P1 | `VopRequiredFrom` | notice |
| 3095 | payee check without result | `VopNoResult` | notice |
| 3270 | instant payment converted to standard | `ConvertedToStandard` | notice; the transfer job reads it (`SentAs::InstantConverted`) |
| 3920 | allowed security functions, P1..n | `AllowedSecurityFunctions` | consumed: TAN method choice |
| 3955 / 3956 | decoupled initiated / still pending | `DecoupledInitiated` / `DecoupledPending` | consumed: decoupled TAN; 3956 means keep polling |
| 3957 | decoupled: wait for the push message | `DecoupledAwaitPush` | notice; as the answer to a status request without 0020 it counts as 3956 |
| 3958 | approval app does not support the procedure | `DecoupledMethodUnsupported` | notice |
| other 3xxx, 1xxx, undefined first digit | warning / note (FinTS 4 only) / undefined | `Other` | notice (3050 and 3075 are such notices) |
| 9000 | status indifferent, P1 order to ask with | `OutcomeUnknown { query }` | category `OutcomeUnknown` |
| 9010, 9050 | processing not possible / message partially faulty | `GeneralError`, `PartialErrors` | generic: lose to specific codes in section 3; `Rejected` |
| 9040 | authentication of the submitter missing or faulty (RM also: decompression error, which this client cannot cause) | `AuthenticationFailed` | `AuthenticationRefused` |
| 9075 | dialog aborted, SCA required | `ScaRequired` | retry with HKTAN (`Retry::SameDialog`), learned per order; with 9800 `Retry::DialogGone` (`tan.rs` `on_sca_required`) |
| 9076 | payments need a name check | `VopRequired` | `Unsupported` |
| 9078 | FinTS product not registered | `ProductNotRegistered` | `Unsupported` |
| 9110 / 9120 | unknown structure / not expected | `UnknownStructure` / `NotExpected` | `ClientFault` |
| 9130-9170 | syntax, length, occupancy, element missing (9150 element not allowed, 9160 element missing) | `ElementNotAllowed`, `DataElementMissing`, else `Other` | `ClientFault` by range |
| 9180 / 9185 | version no longer accepted / protocol version unsupported | `NoLongerAccepted` / `ProtocolVersionUnsupported` | `Unsupported` |
| 9230 | insufficient funds | `InsufficientFunds` | `Rejected` |
| 9250 | name check not possible; submit later or with opt-out | `VopUnavailable` | `Temporary` |
| 9255 | instant limit exceeded, P1 optional link | `InstantLimitExceeded { link }` | `Rejected` |
| 9311 | PIN/TAN system not available | `PinTanUnavailable` | `Temporary` |
| 9340 | signature wrong / faulty / security profile unknown | `SignatureInvalid` | `Credentials(SignatureInvalid)` |
| 9360 | TAN list (or signature) locked after P1 further failed attempts; without P1 "key locked" | `LockAfterAttempts { attempts_left }` | with P1 `Tan`; next to 9941 with P1 > 0 it does not end the order (`Response::tan_rejection`); without P1 `Credentials(Locked)` |
| 9361 | security procedure no longer allowed | `SecurityProcedureNotAllowed` | `Unsupported` |
| 9370 / 9380 | signature authorisation insufficient / user not authorised | `SignatureInsufficient` / `NotAuthorised` | `NotPermitted` |
| 9390 | duplicate submission | `DuplicateSubmission` | `Rejected` |
| 9391 | fetch new customer system ID | `SystemIdInvalid` | `SystemId`; one resync in `jobs::open` |
| 9800 | dialog aborted by the bank | `DialogAborted` | `DialogEnded`; ends the dialog locally |
| 9910 | PIN wrong (RM also: PIN locked, TAN list locked) | `PinWrong` | `Credentials(Unclear)` |
| 9942 | PIN wrong (in a PIN change also: new PIN invalid, too short, too long) | `PinWrong` | `Credentials(PinWrong)` |
| 9930, 9931 | PIN / access locked | `AccountLocked` | `Credentials(Locked)` |
| 9941, 9943, 9951 | TAN invalid / used / timed out | `TanInvalid`, `TanUsed`, `TanExpired` | `Tan`; 9941 as the only cause on a typed TAN keeps the order: `NeedTan` again with `TanChallenge::rejected` ([jobs-step-design.md](jobs-step-design.md) §2, "Wrong TAN") |
| 9959 | SMS not sent | `SmsNotSent` | `Temporary` |
| other 9xxx | error | `Other` | `Rejected` (code and text) |

Seen at banks and typed only as `Rejected` or `Other`: 9210, 9941 (test bank).

## 2. Model

**Scope.** HIRMG codes refer to the whole message (B.7.2); HIRMS codes refer to the customer segment named in the HIRMS header (B.7.3).

```rust
pub struct ReturnCode {
    pub code: Code,                     // four ASCII digits, kept verbatim
    pub text: String,                   // bank text, shown with the code (B.7.5.1)
    pub scope: Scope,                   // Message | Segment { number: u16, order: Option<SegmentType> }
    pub reference_element: Option<String>,
    pub parameters: Vec<String>,
}
impl ReturnCode {
    pub fn class(&self) -> Class;               // Success, Note, Warning, Error, Undefined (first digit)
    pub fn kind(&self) -> Kind;                 // derived from code and parameters
    pub fn category(&self) -> Option<Category>; // class 9 only
}
```

- `Scope::Segment.order` is filled from the sent message's segment numbers (`Response::name_orders`), so a job sees "HKSAL rejected", not "segment 3". A HIRMS without reference has `number: 0`; a reference outside the request leaves `order` `None`.
- An empty parameter is an absent repetition in the codec, so `parameters` holds the non-empty ones in order; positions after an empty parameter shift.
- `Kind` is dataless except where a parameter has a typed reading (`Touchdown`, `UserDataChanged`, `AllowedSecurityFunctions`, `OutcomeUnknown`, `InstantLimitExceeded`, `BicOutdated`, `ProcessingReference`, `VopRequiredFrom`, `LockAfterAttempts`).

**Categories.** `Category` is `#[non_exhaustive]` and `Serialize`/`Deserialize` (snake_case; `Credentials(_)` is an object, `{"credentials":"pin_wrong"}`), one per thing a caller can do differently:

| Category | Codes and errors | Caller reaction |
|---|---|---|
| `Credentials(PinWrong / Locked / Unclear / SignatureInvalid)` | 9942 / 9930, 9931, 9360 without P1 / 9910 / 9340 | ask the user; never retry on your own |
| `SystemId` | 9391 | client resyncs once |
| `ScaRequired` | 9075 | client retries with HKTAN; surfaced only if that is impossible |
| `Tan` | 9941, 9943, 9951, 9360 with P1; `DecoupledMaxPolls` | new TAN from the user (a new order where the bank ended the old one) |
| `DialogEnded` | 9800; `FinTSError::DialogEnded` | open a new dialog |
| `Unsupported` | 9076, 9078, 9180, 9185, 9361; `VersionMismatch`, `AccountNotAddressable` | update client or pick another order |
| `AuthenticationRefused` | 9040 | the bank refused the submitter's authentication; DKB also sends it when the TAN method is locked after too many challenges (user report, unverified). Show the bank text; open no new dialog on your own, since each login asks for a new challenge. Retry when the user asks |
| `NotPermitted` | 9370, 9380 | account or order not allowed for this user |
| `OutcomeUnknown` | 9000; `PaymentOutcomeUnknown` | never resubmit; check status another way (B.7.5.2, RM C.7) |
| `Temporary` | 9250, 9311, 9959; `Transport`, `Timeout`, `Io`, HTTP 5xx/408/429 | retry later unchanged |
| `ClientFault` | 9110, 9120, 9130-9170; `Parse`, `Serialize`, `Dialog`, `DialogPaused`, `WrongResume`, `NoBpd`, `StateFormat`, `PausedDialogMismatch`, `Internal`, `InvalidResponse`, `SegmentUnreadable`, `Mt940`, `Camt`, `Pain002` | bug in this client or its use; report |
| `InvalidInput` | `InvalidTransfer` ([payments-design.md](payments-design.md) section 2) | the caller's data; fix and retry, nothing was sent |
| `Rejected` | every other 9xxx; other HTTP errors | show code and text |

`FinTSError::is_credentials_error()`, `needs_user()` (`Tan`) and `is_retryable()` (`Temporary`) match on `kind()`.

Rules behind the table:

- 9910 is `Unclear`: RM lists it as "PIN wrong" and as "PIN locked" / "TAN list locked", so the bank text tells the user which. It still counts as a credentials error.
- 9340 is `Credentials(SignatureInvalid)`, not "PIN wrong": the PIN may be wrong, or user ID, BLZ or security profile do not match, and banks send it next to a more specific code.
- TAN errors need the user, so `needs_user()` is true for them.
- 9000 is never retryable; `Kind::OutcomeUnknown { query }` names the order to ask with (P1).
- Unreadable HIRMG/HIRMS is `InvalidResponse`.

**Errors.** `FinTSError::Bank(BankError)`; `BankError` has `code()` (the selected code), `codes()` (all codes of the response, HIRMG then HIRMS, warnings included) and `category()`. It displays as `Bank error 9210: text`. `FinTSError::bank_error()` returns it.

**Transfers.** `FinTSError::PaymentOutcomeUnknown { end_to_end_id, message_id }` (`Category::OutcomeUnknown`) is returned for every error after a message that carried a transfer's order or its TAN, except a class 9 rejection (definite, a `Bank` error). That includes transport errors, unreadable answers, an answer that fits no case, a decoupled approval polled past the bank's `max_polls`, and a future dropped mid-request (reported once, by the next call on the dialog). The caller checks the account by those IDs before trying again; the library never resends. `Dialog::cancel` returns `Cancelled::OrderSubmitted { end_to_end_id, message_id }` for a transfer waiting for its TAN, with the same advice ([jobs-step-design.md](jobs-step-design.md), [payments-design.md](payments-design.md) section 3).

## 3. Error selection

When a response has several class 9 codes, `Response::select_error() -> Option<BankError>` picks the reported one; the `BankError` carries all codes of the response.

Three ranks: a **specific** cause ranks above the **signature error** 9340, which ranks above the **generic** codes 9010, 9050, 9800 (summaries or consequences). 9340 has its own rank because banks send it as the consequence of another fault: comdirect answers a BLZ that does not belong to the user with 9050, 9800, 9210 and 9340 in one HIRMG.

1. A credentials code that names the cause (9942, 9910, 9930, 9931), then 9391, wherever they appear (they decide what the caller must do).
2. Otherwise the first specific segment-scope error, in wire order (RM asks banks to put the most important code first, B.7.5.2).
3. Otherwise the first specific message-scope error (e.g. 9210, 9075, 9000).
4. Otherwise 9340, wherever it appears.
5. Otherwise the first error of any kind, segment scope first.

`check_errors` returns the selection. An unreadable HIRMG/HIRMS gives `InvalidResponse`, ranked after credentials and system ID errors and before everything else. The 9391 resync in `jobs::open` and the parameter probe's early stop match on the category.

Test: `protocol::tests::select_error_order` with the live code sets (comdirect 9050 + 9800 + 9210 + 9340 reports 9210; Consorsbank 9800 + 9942 reports 9942).

## 4. Notices

Class 0 and 9 codes are never notices; class 1, 3 and undefined codes are, except the consumed codes in the section 1 table. The protocol consumes those itself (they remain in the trace). B.7.5.1 says to show every bank message with its code, so every other code reaches the caller.

```rust
pub enum Notice {                       // #[non_exhaustive]
    Bank   { dialog: NoticeDialog, code: ReturnCode },
    Client { dialog: NoticeDialog, warning: ClientWarning },
}
```

- `NoticeDialog::{Sync, Job}`: `Sync` is the synchronisation dialog of `jobs::open`, `Job` the dialog the job or login runs in. `Response::notices(dialog)` tags them; `Dialog` tags with `Job` and switches to `Sync` in `sync()`.
- Message or segment level is `code.scope` (HIRMG: `Scope::Message`; HIRMS: `Scope::Segment`, with the answered order's code from the HIRMS reference, e.g. `HKSAL`, `HKTAN`). 3060 and 3072 from the sync dialog and from the job's dialog both reach the caller, so a bug report made from notices keeps where each came from. Within one dialog, a notice identical in dialog, code, scope, text and parameters is added once (`extend_notices`): the TAN and status rounds of one order repeat it.
- A client notice never carries a `ReturnCode`. `ClientWarning::Utf8TextInLatin1`: the HIRMG/HIRMS texts of an answer hold UTF-8 sequences read as Latin-1 (`Response::notices` adds one per answer). `ClientWarning::RangeClamped { requested, used }`: a statement range began before the bank's storage period.
- Identical notices of one dialog are given once, not once per round.
- Accessors: `dialog()`, `code()`, `kind()` (both `None` for a client notice), `client_warning()`. `Display`: `3060 [sync, HIRMG]: text`, `3072 [job, HKTAN]: text`, `3999 [job, HIRMS #7]: text` (segment not in the request), `client [job]: text`.
- `Completed<T> { value, notices }` on `Step::Done` carries the notices of every message the job sent. Those before a `NeedTan` / `NeedVopConfirmation` wait in the resume handle, which is part of the versioned paused-dialog state. `Dialog::take_notices()` hands over what the dialog collected. The notices of a failed operation are on the error's `codes()`.
- `log_informational_codes` writes debug lines for 3091, 3945 and for unknown class 3 codes; the notices are what the caller gets.

## 5. Tests

- `return_code::tests::inventory` (code and params to class, kind), `categories` (code to category), and the consumed-code test `payee_check_codes_are_consumed_only_for_hkvpp_and_hkvpa`.
- `error::tests::classification`: every `FinTSError` variant's category and the predicates.
- `protocol::tests`: `select_error_order`, `outcome_unknown_names_the_order_to_check`, `notices_skip_success_errors_and_consumed_codes`, `notices_name_dialog_and_order`, `utf8_text_in_latin1_answer_is_a_client_notice`.
- `protocol::tests::tan_rejection_only_where_the_order_stays`: which answers to a typed TAN keep the order (9941, 9941 + 9360 P1) and which end it (9210, 9800, 9931, 9360 without P1 or with 0, 9951, 9943).
- Test bank: 9941 the bank does not keep the order for gives `Category::Tan` and `needs_user()`; the retry flows are in `tests/wrong_tan_test.rs`; 3060 and 3072 arrive as notices on the step (`unconsumed_warnings_arrive_as_notices_on_the_step`).
