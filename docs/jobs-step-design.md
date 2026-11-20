# Jobs as steps: design

Rules from [architecture.md](architecture.md) §2.3: every operation returns a step, the library never calls back, the caller owns all state. Code: `src/jobs.rs`, `src/jobs/{accounts,capabilities,paused,transfer}.rs`; tests in `tests/jobs_test.rs` and `tests/jobs_end_to_end_test.rs`.

## 1. `Step<T>`

```rust
#[must_use]
pub enum Step<T> {
    Done(Completed<T>),
    NeedTan(TanChallenge, Resume<T>),
    NeedVopConfirmation(VopConfirmation, Resume<T>), // transfers only (payments-design §2)
    VopPending(VopWait, Resume<T>),                  // transfers only
}
pub struct Completed<T> { pub value: T, pub notices: Vec<Notice> }
```

- **Notices** (return-codes-design §4): every notice of every message the job sent, including those before a `NeedTan`, collected in the pending job and handed out on `Done`. A notice already collected (same dialog, code, scope, text, parameters) is not added twice. Login notices (3920, 3072, ...) arrive on the open step's `Done`.
- **`TanChallenge`**: `challenge` (the bank's text), `challenge_markup` (HITANS "Challenge strukturiert" of the TAN method in use: `challenge` may carry `<br> <p> <b> <i> <u> <ul> <ol> <li> <a href>`), `kind`, `valid_until`, `not_before`, `payee_check`, `rejected`. `TanKind::Typed { media }` or `TanKind::Decoupled { first_wait, next_wait, max_polls, polls_done, auto_polling_allowed }`. `Decoupled` only when the answer carries 3955: `0030` without `3955` is `Typed`, also for a decoupled method (PIN/TAN, TAN-Prozess 4); the waits and `max_polls` are the bank's HITANS v7 values, `None` when it gives none. `auto_polling_allowed == Some(false)` means the user triggers each status request; `poll` does not check it. `not_before` is the arrival of the bank's answer plus `first_wait` (login or order challenge) or `next_wait` (after each 3956); `None` without a bank wait and for typed challenges. It is a wall-clock time and survives freeze and thaw. `payee_check` is a transfer's payee check result next to the TAN prompt, `None` for every other job. `rejected: Option<Box<TanRejected { code: ReturnCode, attempts_left: Option<u32>, count: u32 }>>` says why a typed challenge came back (§2, "Wrong TAN"); `None` on the first ask.
- **`Resume<T>`**: a typed ticket (private `Ticket::{Task(TaskReference), Local(u64)}`; the nonce serves the two payee check waits, which have no task reference). The dialog checks it against its pending job (`Error::WrongResume`). `JobValue` is sealed, one impl per job value.

## 2. The dialog

`Dialog` is the one public dialog type. It holds the transport, credentials, system and dialog ID, message number, `BankParams`, the profile's HKTAN overrides, `phase: Open | Paused(PendingJob) | Ended`, and `last_message_at` (wall clock of the bank's last response). `PendingJob` is plain data: ticket, `Wait::{Tan, VopConfirmation, VopPoll}`, the job and its inputs (touchdown point, pages so far), accumulated notices. It is `Serialize`/`Deserialize` and holds no `Instant`, closure or transport. `PendingJob::{challenge, vop, vop_wait}` read it.

Calls on the dialog:

| Call | Does |
|---|---|
| `submit_tan(resume, tan) -> Result<Step<T>>` | HKTAN process 2 for the task reference (PIN/TAN B.4.2); login or any order. A transfer whose payee check the bank wants released sends HKVPA with the first TAN message. A wrong TAN the bank keeps the order for gives `NeedTan` again ("Wrong TAN" below). |
| `poll(resume) -> Result<Step<T>>` | Exactly one HKTAN process S (process 2 before HKTAN 7). 3956, or 3957 without 0020, gives `NeedTan` again with `polls_done + 1` and a new `not_before`. At the bank's `max_polls` it gives `DecoupledMaxPolls` without sending (`PaymentOutcomeUnknown` on a transfer). On a transfer's `VopPending` it sends the next HKVPP. |
| `confirm_vop(resume)` | Transfers only (payments-design §2). |
| `cancel(resume) -> Result<Cancelled>` | Drops the pending job, sends nothing. `Cancelled::NothingSent` for a read, the login and a transfer at its payee check; `Cancelled::OrderSubmitted { end_to_end_id, message_id }` for a transfer waiting for its TAN (the bank may still execute it). |
| `end(self) -> Result<SessionState>` | HKEND unless the bank ended the dialog; returns the updated state. A paused dialog ends the same way. |
| `session_state(&self) -> SessionState` | A snapshot of the state now (system ID, BPD/UPD, TAN method, learned codes). Sends nothing, changes nothing. The host stores it after `open` and after each job so a crash keeps a new system ID. |
| `capabilities(&[Account])` | §4, sends nothing. |
| `phase()`, `pending()`, `last_message_at()` | Read the dialog. |
| `freeze(self)` | §5. |

`submit_tan` is for typed challenges, `poll` for decoupled ones and payee check waits. The wrong call returns `FinTSError::Dialog` and leaves the job paused.

**No waiting in the library.** It never sleeps, holds no timer, `CancellationToken` or callback. The caller sleeps until `not_before` (its own default when `None`) and decides when to give up. A helper that sleeps would tie the library to a runtime and could not survive a restart: a paused decoupled job is plain data. `standard_run` in `testbank` and the module example show the loop.

### Wrong TAN

**Spec.** PIN/TAN 2020-07-10 (`spec/FinTS_3.0_Security_Sicherheitsverfahren_PINTAN_2020-07-10_final_version.txt`): B.5.1/B.5.2 (HKTAN 6 and 7, "Ausgewählte Beispiele für Rückmeldungscodes") list for HKTAN 9941 "TAN ungültig", 9951 "Zeitüberschreitung im Zwei-Schritt-Verfahren – TAN ungültig", 9931 "Sperrung des Kontos nach x Fehlversuchen", 9210 "Auftrag abgelehnt – Zwei-Schritt-TAN inkonsistent. Eingereichter Auftrag gelöscht" / "Auftragsreferenz ist unbekannt", 9991 "TAN bereits verbraucht"; the archive versions (D, HKTAN 1-5) add 9360 "Sperrung der TAN-Liste nach weiteren x Fehlversuchen". C.2.1 ("Sperre bei mehrmaliger Falscheingabe"): every wrongly signed message (wrong PIN or invalid TAN) raises a failure counter; past the bank's limit the bank locks and says so with 9931. B.4.4.2: a TAN not given in time counts as a failed attempt and the TAN is invalid (9951). Rückmeldungscodes 2026-02-03 B (Fehlermeldungen): 9941 "TAN ungültig" (Bezug: Element), 9943 "TAN bereits verbraucht", 9360 "Sperrung der TAN-Liste nach weiteren %1 Fehlversuchen" (P1: Anzahl), 9931 "Sperrung des Kontos nach %1 Fehlversuchen"; no class 3 warning announces remaining attempts. Formals 2017-10-06 B.7.5.2: a class 9 code rejects its syntactic unit (here the HKTAN, not the stored order); C.1.2: the bank may end the dialog on its own "z. B. bei wiederholter ungültiger Authentisierung des Kunden" with 9800, after which the client sends nothing more in it. **The spec does not say** whether, after 9941, the client may send another HKTAN process 2 with the same Auftragsreferenz; it says neither yes nor no. What it does say: the order is stored at step 1b (B.4.2.1.1), a rejected HKTAN is the rejected unit, and the bank says with 9210 when it has deleted the order and with 9800 when it has ended the dialog.

**Rule.** `Response::tan_rejection` (`src/protocol.rs`): the answer to a typed TAN keeps the order when its class 9 codes are 9941 and, at most, the 9050 summary and a 9360 with attempts left (P1 > 0), and the dialog was not ended (no 9800). Then `submit_tan` returns `NeedTan` with the pending job's challenge (same text, media, validity, payee check) and `rejected` set to the first 9941, 9360's P1 as `attempts_left`, and the count of wrong TANs for this order. The resume ticket is the same task reference; notices of the message are collected as usual. Every other error ends the job: 9210 (order deleted), 9931 / 9360 without attempts (locked, `Credentials(Locked)`), 9951 (timed out), 9943 / 9991 (used), 9800 (dialog ended), unreadable codes. If a bank deleted the order silently after a 9941, the next TAN gets 9210 "Auftragsreferenz ist unbekannt": a class 9 error, so a transfer ends as a rejection.

**Transfers.** A wrong TAN is a class 9 rejection, so the order was not executed: when the bank does not keep the order the error is `FinTSError::Bank` (not `PaymentOutcomeUnknown`; `PaymentIds::after_order` passes class 9 errors through). The library never resends the order (HKCCS/HKIPZ); another TAN goes out only when the caller calls `submit_tan` again, and `cancel` after a rejection is `Cancelled::OrderSubmitted` as for any transfer waiting for its TAN. HKVPA goes with the first TAN message only, as with a decoupled approval: the test bank consumes the VOP-ID with the first TAN message; whether a bank wants it again with the second TAN is not known (banks.md, "Not yet seen at a bank").

**Shape.** A field on `TanChallenge` rather than a new `Step` variant: the caller's reaction is the same as to any typed challenge (show the prompt, take a TAN, `submit_tan`, or `cancel`), with the bank's text in addition. A new variant would make every caller handle a case that is a `NeedTan` in every respect. The field is part of `Wait::Tan` and so of `PendingJob` and `PausedDialog` (format `fints-client-paused/4`): it survives freeze and thaw. `ReturnCode` (not `BankError`) is stored because it is plain serialisable data with code, text and parameters; the value is boxed so `Step` stays small. Decoupled challenges never get `rejected`.

### Phase rules

- **Errors drop the pending job.** A class 9 rejection, a wrong TAN the bank does not keep the order for, or `DecoupledMaxPolls` leaves the dialog `Open` after an order (unless the bank sent 9800) and `Ended` after the login.
- **`Ended`.** 9800 in any answer, a transport, HTTP or parse error, or a dropped future ends the dialog and forgets its ID, so `end` sends no HKEND. `cancel` on the login ends the dialog, and `end` still sends HKEND. `end` on an `Ended` dialog returns the state without sending, so the caller still gets learned codes.
- **Paused.** While `Paused`, any other job returns `DialogPaused` and nothing is sent. After `Ended`, every call returns `DialogEnded`.
- **Not cancel-safe.** The futures of `open`, every job, `submit_tan`, `poll` and `confirm_vop` must run to completion. A future dropped mid-request (`select!`, a timeout around the call) leaves the message number unknown: an in-flight marker stays set, and the next call ends the dialog. If the request carried a transfer's order or TAN, that call returns `PaymentOutcomeUnknown` with the payment's IDs once, then `DialogEnded`; `end` and `freeze` do the same. `session_state` works regardless. Cancel between calls, in the wait until `not_before`; timeouts belong in the transport.
- **Transfer errors.** On a transfer, `poll` past `max_polls` gives `PaymentOutcomeUnknown`, and so does every error after the order went out except a class 9 rejection. The library never resends an order (payments-design).
- **Lifetime.** As long as the bank keeps the dialog (its idle timeout, minutes; not in the BPD). The library sets no expiry; a resume after the bank dropped the dialog fails with `DialogEnded` (9800) and the caller opens a new one.
- **Confirmation.** After process 2 or S the answer is checked and BPD/UPD/3072 are ingested. A read order whose answer lacks its data segment (and 3010) is sent once more, which may pause again. A transfer ends with the TAN answer, its receipt.

## 3. Jobs

Free functions in `jobs`, over an open `&mut Dialog`. Each builds its request through `order_version` first: `NoBpd` or `VersionMismatch` fails before a message is sent, so no TAN is spent on a job that cannot work (version-negotiation-design). A UPD `No` is reported as a capability and sent anyway (bpd-upd-design: expose the allowance, never block).

| Function | Input | Value |
|---|---|---|
| `synchronise(&BankClient, &Credentials, &SessionState)` | as `open` | `Result<Completed<SessionState>>`, not a `Step`: the sync dialog (HKSYN, or the PIN-only parameter probe with a stored system ID) carries no HKTAN. The state has BPD, UPD, system ID and allowed security functions. `InitMode::Direct` (ING) sends nothing and returns the state with the profile's security function. A rejected system ID (9391) is an error here. |
| `open(&BankClient, &Credentials, &SessionState)` | bank client, user ID and PIN | `(Dialog, Step<Opened>)`. Sync dialog first when the state has no system ID or BPD; a stored system ID without BPD is reused through a parameter probe; 9391 re-syncs once. The state's TAN method and medium apply when the bank allows them. A state of another bank or user is ignored (user IDs compared after the profile's completion). `Opened { accounts: Vec<AccountInfo>, tan_methods, allowed_security_functions, selected_tan_method }`. |
| `balance` | `&Account` | `Option<AccountBalance>`; `None` only for a success without HISAL (3010, a notice). A booked balance without date or currency (mandatory in Messages B.4) is kept: `date` is `None`, `currency` is the account currency (HISAL element 4). |
| `transactions` | `&Account`, `Range { from, to }` (inclusive), `Format::{Auto, Camt, Mt940}` | `Statement { booked, pending: Pending, pending_created_at, format }` |
| `holdings` | `&Account`, `Option<Currency>` | `Vec<SecurityHolding>` |
| `depot_transactions` | `&Account`, `Option<Range>` | `Vec<DepotTransaction>` |
| transfers | payments-design | `TransferReceipt`, `InstantStatus` |

`AccountInfo` (UPD account, `src/jobs/accounts.rs`) needs no message (HKSPA is not sent; none of the tested banks needs it).

- **CAMT preferred.** `Auto` sends HKCAZ when the BPD offers a version we build and the UPD does not rule it out (`No` on the matching UPD account, found by its `AccountKey`, so two accounts with one IBAN each get their own entry), else HKKAZ. `Camt` and `Mt940` force the format (`VersionMismatch` for camt without a version, nothing sent). A bank rejecting HKCAZ (class 9) is an error, not a fallback. A bank profile may set `prefer_camt = false`.
- **Pending.** `Pending::Reported(Vec<Transaction>)` (the bank sent a pending part and it parsed, so the list is complete; `Pending::reported()` gives it), `NotSent` (no pending part in the answer), `Unreadable { kind, message }` (the pending part did not parse, or an MT942 `:90D:`/`:90C:` count does not match its bookings, so the list is not complete; a sum or currency mismatch with matching counts is only logged; `FinTSError` is not `Clone`, so category and text are kept). Booked data that does not parse fails the job.
- **Storage period.** `Support::Available::days_stored` is the Speicherzeitraum of the HIKAZS/HICAZS version we would send (0 counts as unknown). A `range.from` earlier than today minus `days_stored` is raised to that day (`to` along when the whole range is older), and the step's notices get `Notice::Client { warning: ClientWarning::RangeClamped { requested, used } }`. "Today" is the date of the dialog's clock: the system's local time unless `BankClient::with_now` pins it (tests and replays; the pin also stamps HNSHK/HNVSK and the pain.001 `CreDtTm`; paused-dialog timing stays on real timestamps).
- **Touchdown paging inside the job.** One call sends pages until no 3040. A repeated touchdown point ends the job with a log line and the bank's 3040 as a notice. Any page may answer with a challenge: the job returns `NeedTan`, and the pending job keeps the pages so far and the touchdown point. The page the TAN confirmed is absorbed (sent again if its answer lacks the data segment) and paging goes on. A later TAN is another `NeedTan`. A later page's error fails the job and drops the pages: a partial statement looks complete to a syncing caller, so retry with a narrower range.
- **Amounts and TAN media.** `Currency::to_minor(Decimal) -> Option<i64>` gives minor units and is `None` on excess precision (never rounded). A method that needs a TAN medium needs its name from the user: the library sends no HKTAB, so it cannot list media.
- **9075 retry and learning** stay inside `send_order`; learned codes reach `SessionState` on `end`/`session_state`.
- **No swallowed errors.** A job completes or fails; no `Option` or empty list stands in for an error. A class 9 rejection leaves the dialog `Open` so the next account's job runs. There is no combined fetch: callers compose jobs and keep each `Result`.
- **Bank-specific init.** `BankConfig::init: InitMode::{Standard, Direct { security_function }}` from the `banks` profile (ING: no sync dialog, stored or client-generated system ID, function 900, no HKTAN); `open` reads it, no branch on the BLZ.

## 4. Capabilities

`Dialog::capabilities(&self, accounts: &[Account])` and `SessionState::capabilities(..)` (the named accounts, or the UPD's when the slice is empty), pure, `Serialize`:

```rust
pub struct Capabilities { pub accounts: Vec<AccountCapabilities> }
pub struct AccountCapabilities { pub key: AccountKey, pub in_upd: bool, pub jobs: Vec<JobCapability>, pub payee_check: Option<VopSupport> }
pub struct JobCapability { pub job: JobKind, pub support: Support, pub upd: Allowance, pub tan_required: Option<bool> }
pub enum Support {
    Available { code, version, formats, days_stored },
    NotOffered,                                // no parameter segment in the BPD
    VersionMismatch { offered, implemented },
    NoBpd,
}
```

`JobKind` is the public job enum (`ALL` lists nine: balance, transactions, holdings, depot transactions, credit card, depot history, transfer, instant transfer, instant status); `AccountCapabilities::job(JobKind)` looks one up. `upd` is the allowance of the order that would be sent; it is information, never blocking. `tan_required` is the `order_tan` result (override, learned code, HIPINS), `None` when nothing says. `Transactions` reports HKCAZ or HKKAZ by the rule `jobs::transactions` uses. The session variant lacks the client's TAN overrides (they live in `BankConfig`). A version 0 UPD (dialog-only, Consorsbank) is not stored: `SessionState::capabilities(&[])` has no accounts, and with named accounts every allowance is `Unknown` and `in_upd` false; `Dialog::capabilities` has the UPD.

## 5. Frozen dialogs

`Dialog::freeze(self) -> Result<PausedDialog>` consumes the dialog (no live copy can send the same message number) and sends nothing; it works for an open dialog between jobs and for one paused on a TAN. `PausedDialog` holds everything but transport and PIN, plus the dialog's own BPD/UPD and TAN selection, so it is self-contained (session-state-design §3 and §4). `Dialog::thaw(PausedDialog, &BankClient, &Credentials) -> Result<Dialog>` checks BLZ, URL and user ID (`PausedDialogMismatch`). `PausedDialog::pending() -> Option<AnyResume>` (one variant per job) rebuilds the typed ticket after a restart; `AnyResume::kind() -> ResumeKind` names the job without matching on the value. Thaw never refuses on time: a dialog the bank dropped answers `DialogEnded`.

## 6. Tests

Test bank, in process, each asserting the `Step` sequence, values and notices: login exemption (3076); decoupled login (3956 twice, then 0020; `polls_done`; `DecoupledMaxPolls` without a request); typed TAN for login and order; 9075 learning; CAMT, MT940 by UPD, version or not offered, no fallback on HKCAZ 9xxx; touchdown with a TAN on page 2 only (`TanRule::OnPage(n)`); wrong TAN then right TAN for login, read and transfer (executed once, HKVPA with the first TAN only), 9360 attempts, wrong TANs until the bank deletes the order (9210) or ends the dialog (9800), freeze and thaw between the wrong TAN and the retry (`tests/wrong_tan_test.rs`; `testbank::WrongTanModel`); pre-send `VersionMismatch`; balance rejection while transactions still succeed; faults (9800 mid-paging, cut wire) leave the dialog `Ended`; `DialogPaused`, `WrongResume`; job futures are `Send`. Capabilities: unit tests over BPD/UPD fixtures and `capabilities_match_what_the_jobs_do`. Replay: `replay_test.rs` and the `.expect` sidecars.
