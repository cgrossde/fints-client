# DKB

Part of the [bank list](../banks.md). Anything not confirmed by a live run or the bank itself is marked **unverified**.

Bank code 12030000, server `https://fints.dkb.de/fints`, BIC `BYLADEM1001`.

## TLS

- The server certificate chains to *Sectigo Public Server Authentication Root E46*. Older trust stores don't include it, so the connection fails with a certificate error.
- For fints-client: rustls with current root certificates (or the OS trust store) avoids this.

## Login and SCA

- Decoupled approval in the DKB app at login (3955, 3956, then 3920/0900; trace 2026-11-17): one approval per dialog, approved after the first status poll.
- **Every new dialog is a new push.** Our live runs got `3955` on every login, not `3076`. User reports conflict on whether a stored session logs in again without approval; the exemption is the bank's choice. Do not promise a sync without approval, and poll a pending login with HKTAN S in the same dialog rather than opening a new one.
- **`9040` after too many challenges** (user report, **unverified**): DKB answers `9040` when the TAN method is locked for the day after too many requested challenges; in bad cases the app session ended too. The library reports it as `Category::AuthenticationRefused`: do not start another login on your own.
- A 3920 that allows only 910 and 913 (chipTAN) means the user has no app device registered (user report 2025-02, **unverified**). The library then selects chipTAN; the user needs a generator or must register the app.
- The BPD offers HITANS 6 (910, 913) and 7 (940, 910, 913); 3920 allows only 940 (live 2026-11-17). The client picks HKTAN 7, the highest version listing the chosen method (PIN/TAN B.4); seen live, no risk. The test bank model offers both.
- The synchronisation dialog goes without HKTAN, the personalised init then sends `HKTAN:5:7` with HKIDN: seen live 2026-11-17.

## Account list (UPD)

- UPD version 0 (dialog-only): every dialog receives it again (trace 2026-11-17). Kontoart on every account.
- **No BIC** on any account. Fill it from the bank code.
- **Credit cards have no IBAN.** The UPD entry (account type 50) has a 16-digit account number and a 30-character subaccount token. The bank allows HKSAL, HKKAZ and HKCAZ for the card, but HKSAL v8 and HKCAZ cannot address it by IBAN. DKB doesn't offer the proprietary DKKKU (no `DIKKUS` in the BPD) either.
- **Nothing in the UPD links a card to its settlement account.** All accounts share one customer ID. The card's bookings appear on the current account it settles against.
- **Card addressing: national-only KTI (live 2026-11-18).** HISPAS 1-3 say `N:J:N` (national account data allowed). With that, the client sends KTI as `::<number>:<sub>:280:<blz>` (IBAN and BIC empty; Messages B.3.2) for HKSAL 8 and HKCAZ 1, and refuses with `AccountNotAddressable` when HISPAS says `N`. The checking accounts take the six-part KTI. In the 2026-11-18 trace the card (account #5, Kontoart 50, no IBAN) answered HKSAL 8 and HKCAZ 1 with 0020: balance and transactions were read, no 9xxx.
  - Open: whether the card's bookings duplicate those of its settlement account.
- BPD job versions seen: HISALS v6/v7/v8, HIKAZS v6/v7, HICAZS v1.

- **Loan accounts** have not been delivered over FinTS since DKB's server migration in 2024-11 (user report, **unverified**).

## Account types

- DKB sends FinTS account type 1 (current account) for Tagesgeld accounts as well. Only the product name (`Tagesgeld`) tells them apart. Credit cards have type 50.
- No overdraft limit in the UPD.

## Balances

- Booked balance and credit line. No available balance, no pending balance.

## Transactions

- **Storage period:** HIKAZS v6/v7 360 days, HICAZS v1 450 days (live trace). The library clamps an older `from` to it (`ClientWarning::RangeClamped`).
- CAMT (HKCAZ) works and includes pending bookings. Every booking carries the structured SEPA fields: counterparty IBAN/BIC, end-to-end reference, mandate reference, creditor ID, purpose code (ISO 20022, e.g. card payment, standing order).
- The HICAZ answer is a camt.052.001.08 document (requested `.08`, live 2026-11-17); comdirect's answer is a .08 document too, so the .08 field mapping is seen at two banks.
- The purpose arrives as one line, not split into 27-character fragments as in MT940.
- Pending bookings often lack the counterparty IBAN (card payments).
- The bank's own booking reference is present and unique on every booking of the current accounts, pending ones included. It sits at entry level (`Ntry/AcctSvcrRef`), never in `TxDtls/Refs`, and each entry has exactly one `TxDtls` (fints-client trace 2026-11-17, shape only: the values are masked, so uniqueness could not be rechecked there). The credit card's camt entries carry no `AcctSvcrRef` at all (same trace, 16 booked and 3 pending). **Unverified** whether a pending booking keeps its reference when it gets booked.
- Not sent: ultimate debtor/creditor, original amount and currency, fees.
- Live 2026-11-19: 5 accounts, 183 bookings since 2026-10-01. A pending booking can come without a value date (one of two). A booking without `Ustrd` has no purpose: fints-client does not take the booking text (`AddtlNtryInf`) as one.

## For the bank profile

- Profile entry (`src/banks/table.rs`, 12030000): `booking_id = BookingIdSource::CamtEntryAcctSvcrRef`. A booked camt entry (`Sts` `BOOK`) gets its entry-level `AcctSvcrRef` as `Transaction::booking_id`. Pending entries get `None`, because a pending booking does not keep its reference when it is booked. Card bookings get `None` (no reference). MT940 (HKKAZ) gets `None`. A reference on two entries of one statement is dropped from both. `bank_reference` is unchanged. Tests: `booked_entries_get_their_entry_reference_as_booking_id` (`src/camt.rs`), `statement_booking_ids_follow_the_rule_on_booked_camt_only` (`src/jobs.rs`), the DKB run in `tests/bank_profiles_test.rs`.
  - Seen live (hashed dumps of 2026-11-19 and 2026-11-20, compared by hash): the two bookings pending on the first day were booked on the second, each under a new `AcctSvcrRef`; neither pending reference appeared again. The booking date changed too, and one of the two (matched by amount alone) also came with another counterparty name. The references of the 181 booked entries of the first day were all unchanged on the second.
- Offline model: `testbank/src/profiles.rs` (`dkb()`); behaviour test `dkb_standard_run_sends_hksal_8_and_hkcaz_1_per_account` in `tests/bank_profiles_test.rs` (decoupled login, HKSAL 8 and HKCAZ 1 with HKTAN, the card with the national-only KTI).
- Credit cards without IBAN: the library sends the national-only KTI when HISPAS allows (generic rule in `kti`, no profile entry); the caller may still skip them.
- Missing BIC: generic, filled from the directory by bank code.
- Classify savings accounts by product name when the account type says current account: the caller's job, not the library's.
- The caller's job: de-duplicate booked rows by `booking_id`. The pending list of each sync is complete (`Pending::Reported`): replace the stored pending rows with it rather than matching them, since a booking gets a new reference, date and sometimes counterparty name when it is booked.

## Payments (live 2026-11-18, 1.00 EUR test)

- **Card read:** the Visa credit card (Kontoart 50, no IBAN) answers HKSAL and HKCAZ with `0020` and HKTAN with `3076` (national-only KTI, as for the other accounts): balance and 16 transactions. The UPD allows HKCAZ, HKKAZ and HKSAL for it.
- **Login:** `HKIDN + HKVVB + HKTAN(4, HKIDN)` gets `3955` and `0030`; the status requests HKTAN(S) get `3956` twice, then `3920` + `0900`. Every answer carries HIRMG `3060` and `0010`.
- **Reads per account:** `HKSAL:8 + HKTAN(4, HKSAL)` and `HKCAZ:1 + HKTAN(4, HKCAZ)` get `0020` on the order and `3076` on HKTAN.
- **Transfer (HKCCS):** message 1 `HKVPP:1 + HKCCS:1 + HKTAN(4, HKCCS)` is answered `3090` on HKVPP and `3945` on HKTAN. The result comes as pain.002 with `RVMC` (close match) and the registered name. Message 2 `HKVPA:1 + HKCCS:1 + HKTAN(4, HKCCS)` gets `0010` on HKVPA, `3955` and `0030` on HKTAN; the decoupled status request HKTAN(S) gets `0010`. Receipt status `Accepted`.
- **Second run:** one letter of the name mistyped also gave `RVMC`. The user cancelled at the confirmation; the client sent HKEND and nothing was executed.
- **No match:** a clearly wrong payee name gave `RVNM` in the same shape (`3090` on HKVPP, `3945` on HKTAN). The user cancelled at the confirmation; the client sent HKEND, answered `0100`, and nothing was executed.
- **Clean match (live 2026-11-20):** message 1 `HKVPP:1 + HKCCS:1 + HKTAN(4, HKCCS)` gets `3090` and `0025 Keine Namensabweichung` on HKVPP and `3945` on HKTAN; the pain.002 result is `RCVC`. The bank still wants the second message: `HKVPA:1 + HKCCS:1 + HKTAN(4, HKCCS)` gets `0010` on HKVPA, `3955` and `0030` on HKTAN; the status request HKTAN(S) gets `0010`. The transfer was executed.
- **No match again (live 2026-11-20):** `RVNM` with `3090` alone on HKVPP (no `0025`); cancelled at the confirmation, HKEND, nothing executed.
- **Not seen yet:** `RVNA`, an instant payment (HKIPZ).
- HKCCS 1 with pain.001.001.03 and .001.09; HKIPZ 1 and 2 with pain.001.001.09 only (conversion to a standard transfer not allowed); HIIPSS minimum wait 5 seconds. HIPINS: HKCCS and HKIPZ need a TAN, HKIPS, HKVPP and HKVPA do not.
- Payee check (HIVPPS): delivery `Complete`, plain explanation text, result as pain.002.001.10 (URN), required for 14 orders including HKCCS and HKIPZ.
- TAN method: decoupled (`DKB App`); the confirmation of a payment is polled with HKTAN.
- Offline model: `profiles::dkb()` (`vpa_answer` 0010); test `dkb_close_match_confirms_then_decoupled_tan` in `tests/payments_test.rs`.
