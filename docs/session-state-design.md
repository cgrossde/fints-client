# Session state and paused dialogs

The caller owns all state ([architecture.md](architecture.md) §1 and §2.2). Two values carry it: `SessionState` between dialogs and `PausedDialog` inside one. Both are versioned, serialisable data; the library writes no files. `Dialog`, `Step` and `Resume<T>` are in [jobs-step-design.md](jobs-step-design.md) §5.

## 1. SessionState

`src/session.rs`. Fields:

- `format` (private tag), `blz`, `user_id`, `system_id` (public, so a host that stores the ID itself can set it), `bpd_version`, `upd_version`.
- Raw `bpd` and `upd` as wire strings. A dialog-only UPD (version 0) is not stored.
- `tan_method`, `tan_medium`, `allowed_security_functions`, `previous_user_id`, `tan_required_learned`, `updated_at`.

`Dialog::end` returns the updated state. `Dialog::session_state(&self)` returns a snapshot of it: it sends nothing, changes nothing, and works while a payment request is in flight. A host stores it after `open` and after each job, so a crash keeps a new system ID.

`SessionState::with_system_id` starts from a known system ID. After a format change the new state has no BPD/UPD, and one parameter dialog fetches them; the bank's device entry stays.

## 2. Format

- **Envelope.** Each stored type carries `format` as a string, as the trace does: `fints-client-session/2` (`SESSION_STATE_FORMAT`) and `fints-client-paused/4` (`PAUSED_DIALOG_FORMAT`). The tag is a unit type whose `Deserialize` accepts exactly its string, so the check holds for every serde format. Any other value, older or newer, fails; `from_json` maps that to `FinTSError::StateFormat { found, expected }`. No migration, no `serde(default)`, `deny_unknown_fields`. The number changes with any change to the shape.
- **Encoding.** Both types derive `Serialize`/`Deserialize` (the caller picks JSON, CBOR, ...) and offer `to_json`/`from_json` as the default.
- **Never stored:** PIN, TAN, transport, quirk chain, product ID, any `Instant`. The bank client (URL, profile, product ID) and the PIN are passed in again on `thaw`.
- **Stored but sensitive:** user ID, system ID, UPD (IBANs, names), dialog ID, task reference, challenge text and HHD-UC/photo data (may name payee and amount), and for a paused transfer the payee names, IBANs and amounts. The module docs say "store encrypted"; integrity is the caller's storage.
- **Debug** is hand-written for both types: BLZ, versions, segment counts, phase, `last_message_at`; user ID, system ID and dialog ID as stars (one per character); no UPD content, no challenge text.

## 3. Paused dialog

`src/jobs/paused.rs`. A `PausedDialog` holds:

- `format`, `blz`, `url` (checked against the client on thaw), `user_id` (checked against the credentials), `system_id`, `dialog_id`, `message_number` (the next one to send).
- `params`: raw BPD/UPD in wire form including a dialog-only UPD, their versions, selected and preferred TAN method, TAN medium, allowed functions, `changed_user_id`, learned codes.
- `tan_overrides`: the profile's HKTAN overrides at open. Profile changes apply to new dialogs only.
- `pending: Option<PendingJob>`, `None` for a dialog frozen while open.
- `last_message_at`: wall clock of the last response (t0 in Formals, life indicator, Abb. 13).

The pending job is `{ ticket, wait, job, notices }`. `ticket` is `Task(TaskReference)` or `Local(u64)` (payee check waits). `wait` is `Tan(TanChallenge)`, `VopConfirmation(VopConfirmation)` or `VopPoll(VopWait)`. `job` holds the inputs, HKTAN version and process, touchdown point and pages so far. A paused transfer stores its pain.001 verbatim (a UTF-8 string, so the execution order resends the same bytes), its payee check state (VOP-ID, polling ID, touchdown, pain.002 parts, result, explanation) and, while the user decides, the still-live challenge of message 1.

**Self-contained.** The paused dialog carries its own parameter data instead of pointing at a `SessionState`, because a dialog-only UPD (Consorsbank) is in no `SessionState`. The caller stores one value per paused dialog. `end()` on the thawed dialog returns the updated `SessionState`.

**Expiry.** `expires_at()` is `last_message_at` plus HIBPA DE 10 "Maximaler Timeout-Wert" (seconds; absent or 0 = no limit, Formals data dictionary), from our own clock only. It is informational: `thaw` never refuses on time, and the bank answers. An unknown or timed-out dialog gets 9800 (`DialogEnded`); a TAN past its window gets 9951 (PIN/TAN B.4.4.2), the TAN error variant. The bank may end a dialog earlier at any time (Formals, life indicator). The HITAN challenge validity (PIN/TAN "Gültigkeitsdatum und -uhrzeit für Challenge") is a zone-less bank-local value; `TanChallenge::valid_until` keeps it as a civil `DateTime`, read from every HITAN version, with no zone and no conversion. `not_before` (decoupled challenges) also survives the round trip.

## 4. API

```rust
impl Dialog {
    pub fn session_state(&self) -> SessionState;
    pub fn freeze(self) -> Result<PausedDialog>;
    pub fn thaw(p: PausedDialog, client: &BankClient, creds: &Credentials) -> Result<Dialog>;
}
impl PausedDialog {
    pub fn pending(&self) -> Option<AnyResume>;
    pub fn challenge(&self) -> Option<&TanChallenge>;     // TAN-only, no thaw needed
    pub fn vop(&self) -> Option<&VopConfirmation>;        // the confirmation to show
    pub fn blz(&self) -> &Blz;
    pub fn last_message_at(&self) -> Timestamp;
    pub fn expires_at(&self) -> Option<Timestamp>;
    pub fn to_json(&self) -> Result<String>;
    pub fn from_json(s: &str) -> Result<Self>;
}
impl AnyResume { pub fn kind(&self) -> ResumeKind; }
```

- **`freeze`** consumes the dialog, so no live copy can send the same message number. It works on `Open` and `Paused` and sends nothing. An ended dialog fails with `DialogEnded`; a dialog whose transfer request was dropped fails with `PaymentOutcomeUnknown`.
- **`thaw`** checks, before sending anything: format (on deserialize), `blz` and `url` against the client, `user_id` against the credentials (else `PausedDialogMismatch { field }`, a `ClientFault`). It builds the protocol dialog through the client (its transport, product ID, default quirks) with the stored parameters, dialog ID and message number. The next message carries the same dialog ID and `message_number`. The thawed dialog is `Paused` on the same job, or `Open`.
- **`AnyResume`** has one variant per job: `Open`, `Balance`, `Transactions`, `Holdings`, `DepotTransactions`, `CreditCard`, `DepotHistory`, `Transfer`, `InstantStatus`, each holding the typed `Resume<T>`. `kind()` returns the matching `ResumeKind` without matching every variant. Take `pending()` before `thaw`, which consumes the value.

`examples/sync_accounts.rs` shows the flow: at a TAN prompt `dialog.freeze()?.to_json()?`; on the next run `PausedDialog::from_json`, `pending()` for the ticket, `Dialog::thaw`, then `poll`/`submit_tan` by variant.

## 5. TAN method choice

A new state has no BPD, so the caller cannot see the allowed methods before a login. `jobs::synchronise(&client, &creds, &state) -> Result<Completed<SessionState>>` runs only the sync dialog (or the PIN-only parameter probe with a stored system ID), no login and no TAN. Its state lists `tan_methods()` and `allowed_security_functions`. The caller sets `with_tan_method` (and `with_tan_medium` for a method with `needs_tan_medium`) and passes the state to `jobs::open`, which skips the sync dialog and logs in with that method. With `InitMode::Direct` (ING) `synchronise` sends nothing and sets `tan_method` to the profile's security function; `open` uses that function whatever the caller sets.

`open` and `synchronise` accept a state whose `user_id` is the short form the profile completes (Consorsbank's 9 digits). Both IDs are compared completed, so `SessionState::new(&blz, &short_id).with_tan_method(..)` keeps its preference; the state after the dialog holds the completed ID.

## 6. Tests

- Unit (`src/session.rs`, `src/jobs/paused.rs`): JSON round trip of both types, including a dialog-only UPD in `PausedDialog`; other tags and an unknown field fail; `Debug` hides identifiers, UPD and challenge; `expires_at` with a 300 s timeout, with 0 or absent, and without the challenge validity; `AnyResume::kind`.
- Test bank, in-process (`tests/paused_dialog_test.rs`), each through JSON into a new `BankClient`: decoupled login frozen before and after a poll; typed TAN on balance; TAN on touchdown page 2 (page 1 kept); an open dialog between two balances; Consorsbank's dialog-only UPD; mismatch (user, BLZ, URL); freeze after `cancel` of the login fails with `DialogEnded`; `Debug` of a real dialog without PIN, user ID, system ID or IBAN.
- Not covered: a bank that drops the dialog and answers 9800 after thaw. The test bank treats a message to an unknown dialog as a protocol violation.
- Freeze sends nothing, so replay fixtures are unaffected.
