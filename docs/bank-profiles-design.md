# Bank profiles: design

Rules from [AGENTS.md](../AGENTS.md) ("Banks"): bank-specific behaviour is a profile in `banks` plus an entry in [banks.md](banks.md), never a branch on the bank code in `protocol` or `jobs`. A profile holds only what the bank's parameter data (BPD, UPD) cannot tell the client. Jobs are described in [jobs-step-design.md](jobs-step-design.md).

## 1. Model

- **`BankConfig`** (`src/banks/mod.rs`): `{ name, blz, bic, url, profile: BankProfile }`. `new` gives the generic profile. `with_url` and `with_tan_override` keep the rest of the profile; `with_profile` replaces it.
- **`BankProfile`** (`src/banks/profile.rs`): owned, serde data.
  - `id: ProfileId` (`generic`, `dkb`, `comdirect`, `consorsbank`, `ing`; for logs, traces and bank-specific help).
  - `init: InitMode`: `Standard`, or `Direct { security_function }` (no synchronisation dialog; the business dialog opens PIN-only and the bank sends its parameters there).
  - `tan_overrides: TanOverrides`: orders for which the profile fixes whether HKTAN is sent, beating HIPINS. Empty in every shipped profile: `9075` learning covers a wrong HIPINS (Consorsbank), and an override needs a trace or a banks.md observation.
  - `user_id: Option<UserIdRule>`.
  - `booking_id: BookingIdSource`.
- **Lookup.** `bank_by_blz` is the generated directory entry (name, BIC, URL per BLZ) plus `profile_by_blz(blz)`; `all_banks` does the same. `profile_by_blz` gives the table entry, or generic for any other code. The `testbank` feature resolves BLZ 12345678 to the local test bank.
- **Table** (`src/banks/table.rs`, `PROFILES`): explicit BLZ lists, each with a constructor. One URL can serve several banks, so a URL is not an identity; a unit test checks that every directory code sharing a profiled bank's URL is listed.

| Profile | Codes | Fields |
|---|---|---|
| dkb | 12030000 | `booking_id: CamtEntryAcctSvcrRef` |
| comdirect | 20041111, 20041133, 20041144, 20041155, 20041166, 20041177, 20041188, 20041199 | none |
| consorsbank | 76030080, 70120400 (former DAB code, same host) | `user_id`: 9 or 10 digits, suffix `001` |
| ing | 50010517 | `init: Direct { "900" }` (**unverified**, banks/ing.md) |

An entry needs a trace or a banks.md observation where the generic rule gives the wrong result. Notes live in banks.md, cited from the profile's doc comment.

- **Directory.** `build.rs` reads `data/fints_institutes.csv` (Windows-1252, `;`), keeps rows with an 8-digit BLZ and an `http` PIN/TAN URL, first row per BLZ wins, and emits `bank_directory.rs`, a sorted table that `banks` searches by bank code. comdirect has eight codes, each with its own BIC and the same URL.
- **Replay and recording.** `HttpReplay::bank_config` and `examples/record_trace.rs` (`FINTS_URL`) use `bank_by_blz(blz)?.with_url(url)`, so a replay uses the profile the live run used. A BLZ outside the directory gets `BankConfig::new` (generic profile). `HttpBank::bank_config` and `BankHandle::client` take the model's name, BLZ and BIC and `profile_by_blz(model BLZ)`.

## 2. Generic rules (no profile needed)

Each is confirmed in a trace:

- BIC from the bank directory when the UPD has none (`Account::from_sepa`).
- National KTI parts when HISPAS allows them (`segments::request::kti`).
- Dialog-only UPD (version 0).
- `9075` retry and learning.
- Job versions and camt or MT940 from the BPD (`order_version`, `choose_statement_format`).
- Decoupled login from HITANS and 3955.
- Account classification (3.2).

`QuirkChain::default()` holds the four tolerant repairs; no bank needs a rewriter.

## 3. Rules

### 3.1 User ID

`UserIdRule { min_digits, max_digits, suffix }`. A user ID of only ASCII digits whose length is in range gets `suffix` appended; anything else (letters, other lengths, empty) is sent as given (`UserIdRule::apply`).

Consorsbank: the FinTS user ID is the account number plus `001`, and the request fails without it (banks/consorsbank.md, "FinTS user ID"). The library applies the rule, not the caller, because a wrong guess costs a PIN attempt and every caller would otherwise re-read banks.md.

`BankClient::effective_user_id` (debug log, masked) is used by `new_dialog` (every dialog, so the HKIDN, HNVSK and HNSHK user ID), by `jobs::open` (the session-state match) and by `Dialog::thaw` (the paused-dialog match). `SessionState` and `PausedDialog` store the completed user ID, so a thaw with the short form still matches. Completion is idempotent. The recorded traces mask the user ID, so replays do not show it.

comdirect has no rule: a login with another bank code answers 9210 (banks.md).

### 3.2 Account classification

`SepaAccount::classify() -> Option<(AccountClass, ClassSource)>`, `ClassSource::{Kontoart, AllowedJobs}` (`src/types.rs`).

- Kontoart 1-99 by the ranges of the Formals data dictionary; a value outside 1-99 counts as no Kontoart.
- No Kontoart: depot when HKWPD is allowed and none of HKSAL, HKKAZ, HKCAZ is. comdirect and Consorsbank show this UPD pattern. The rule does not call a clearing account that also lists HKWPD a depot.
- Otherwise `None`; nothing is guessed as checking.

`is_depot()` is `classify()` = Depot. `product_name` and `account_type` carry the bank's values unchanged; reclassifying by product name (DKB Tagesgeld) is the caller's job.

### 3.3 Booking ID

`BookingIdSource::{None, CamtEntryAcctSvcrRef}` feeds `Transaction::booking_id` through `parse_camt052(xml, BookingIdSource)`. `CamtEntryAcctSvcrRef` takes the entry-level `Ntry/AcctSvcrRef` of booked entries only; pending entries, MT940 statements and entries without the element get no ID. The MT940 `:61:` bank reference is never one (often `NONREF` or repeated). DKB: the reference is unique per booking (banks/dkb.md); whether a pending booking keeps it is unverified.

### 3.4 Not profile entries

Job versions, camt vs MT940, HKTAN per order, KTI form, decoupled vs typed TAN and UPD storage come from the BPD and UPD. DKB's credit card (no IBAN, KTI national-only) is a generic codec item driven by HISPAS national `J`, not a profile entry. Skipping a card is the caller's decision; the library refuses before sending (`AccountNotAddressable`).

## 4. Evidence from the traces (`testbank/fixtures/replay/`)

| | DKB 12030000 | comdirect (one of 20041111-99) | Consorsbank 76030080 |
|---|---|---|---|
| Login | decoupled: 3955, 3956, then 3920/0900 | 3076 (no approval) | 3076 |
| Orders | HKSAL 8, HKCAZ 1, each with HKTAN, 3076 | HKSAL 5, HKCAZ 1 with HKTAN, 3076; HKWPD 5 without | HKSAL 6, HKKAZ 7: 9075 once each, learned; HKWPD 6 without |
| HIPINS | HKSAL/HKKAZ/HKCAZ `J`, HKWPD `N` | HKSAL/HKKAZ/HKCAZ `J`, HKWPD `N` | HKSAL/HKKAZ/HKWPD `N` (wrong for HKSAL/HKKAZ) |
| HISPAS national | `J` | `N` | `J` |
| UPD | version 0, Kontoart on every account; card (50) has no IBAN and gets no request | version 1, no Kontoart; depot allows HKWPD, no HKSAL/HKKAZ/HKCAZ | version 0, no Kontoart; depot allows HKWPD, HKPRO only |

## 5. Tests

- **Unit** (`src/banks/mod.rs`): every profiled code is in the directory; no code in two profiles; every directory code at a profiled bank's URL is listed (comdirect: eight); `bank_by_blz` and `all_banks` agree; ING `init`; an unprofiled code gives generic; `with_url` keeps the profile; no shipped TAN overrides; a profile round-trips through JSON; `consorsbank_completes_short_user_ids`.
- **Test bank models** (`testbank/src/profiles.rs`: `consorsbank()`, `dkb()`, `comdirect()`; in-process only, so a real bank code never reaches the network). Each rule cites its evidence; customer and accounts are synthetic. `tests/bank_profiles_test.rs` runs `standard_run` against them and asserts the request sequence of section 4, the account classes and the profile id from `bank_by_blz`; it includes DKB's card refused before sending and comdirect's 9210.
- **Evidence tests** compare each model's BPD with the committed trace of its bank code, `<bank>-<BLZ>.trace.json`, and under `REPLAY_LOCAL=1` also with a newer recording in `local/`. A trace is committed after the maintainer's review ([trace-tooling.md](trace-tooling.md), section "Before committing a trace").
- **Replay.** `REPLAY_LOCAL=1 cargo test --test replay_test` runs the three local traces.
- **User ID** (`tests/jobs_test.rs`, test bank): `short_user_id_is_completed_on_the_wire_and_in_the_state`, `thaw_with_the_short_user_id_matches_the_completed_one`.

## 6. Out of scope

- DKB credit card: KTI with national parts and no IBAN in HKSAL 7/8 and HKCAZ. Generic and BPD-driven; live 2026-11-18: HKSAL 8 and HKCAZ 1 answered 0020 for the card ([banks.md](banks.md#dkb)).
- comdirect's thin clearing-account bookings (display fallback): a caller concern, banks.md only.
- Bank-specific quirks: none observed.
