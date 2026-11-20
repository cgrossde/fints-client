# Consorsbank

Part of the [bank list](../banks.md). Anything not confirmed by a live run or the bank itself is marked **unverified**.

Bank code 76030080, server `https://brokerage-hbci.consorsbank.de/hbci`, BIC `CSDBDE71XXX`. The directory also lists 70120400 (BIC `DABBDEMMXXX`, the former DAB) at the same server; the profile covers both.

## FinTS user ID

- **The user ID is the account number plus a 3-digit user number** (*Berechtigtennummer*): 9 digits (10 for former DAB customers) followed by `001` for the first account holder. Further authorised users get `002`, `003`, and so on. The web login accepts the account number alone, so customers type that and get `9942 PIN ungültig`.
- Sources: public help pages of banking apps; not confirmed by Consorsbank itself (**unverified**).
- A 2026-01 user report quotes the bank's HBCI FAQ differently: account number padded with a leading zero to 10 digits, then `001` (13 digits), and the PIN is the 5-digit online PIN, not the web password (**unverified**). The rule below stays as it is; a 13-digit ID passes through unchanged.
- **The library completes the short form** (the rule's source is still unverified): a user ID of only ASCII digits, 9 or 10 long, gets `001` appended before it is sent; anything else (12 or 13 digits, letters, other lengths) is sent as given. Co-holders (`002`…) type their full ID. A user who types the short form sends the completed ID; the short form itself gets `9942`. `SessionState` and `PausedDialog` store and compare the completed ID, so a state or paused dialog recorded with the full ID matches a later call with the short form. Debug log, user ID masked. Rule: `BankProfile.user_id` (`UserIdRule::apply`), tests in `src/banks/mod.rs` and `tests/jobs_test.rs`.

## Login

- A wrong PIN gives `9800 Der Dialog wurde abgebrochen.` and `9942 PIN ungültig.` in the answer to the first message (seen live 2026-11-17). The client reports 9942 (`Category::Credentials(PinWrong)`).

## TAN methods

- Offered (HITANS, live trace): 900 *SecurePlus TAN Generator* (photoTAN, `MS1.0.0`) and 901 *Consorsbank/myPrivateBank App* (decoupled, `MS2.0.0`).
- The SecurePlus **app** was shut down on 2026-04-25; its TANs now get `9941` (user report 2026-06, **unverified**; an earlier note dated this 2025-08). The 900 generator is a separate device.
- Per a 2026-06 user report, 901 asks for a **typed** 9-digit TAN at login when the bank's SCA exemption has lapsed, and app approval only for orders (**unverified**; our live logins got `3076`). The library takes a challenge as decoupled only on `3955`: `0030` without `3955` is a typed TAN, whatever the method.

## SCA on reads: the BPD contradicts the server

- HIPINS marks HKSAL, HKKAZ and HKWPD as **not** needing a TAN (`needtan=N`). Dialog init gets `3076` (no SCA needed).
- Sent without HKTAN, HKSAL v6 and HKKAZ v7 fail with `9050` + `9075 Starke Authentifizierung erforderlich`.
- **Workaround:** send them with HKTAN anyway. The bank then answers `3076` and returns the data, with no app approval. Doing the same for HKWPD fails with `9160` (missing order reference), so HKWPD goes without HKTAN and works.
- The client retries once with HKTAN on `9075` and learns it per order (session state), without a profile entry; with `9800` next to `9075` it learns and returns the error. Confirmed live 2026-11-16: Consorsbank sends `9075` with `9050` and no `9800`; the retry in the same dialog gets `3076` and the data, and every later HKSAL/HKKAZ carries HKTAN at once.
- Profile mechanism: `BankProfile.tan_overrides` (per order, `Always` or `Never`, beats HIPINS) and the table in `src/banks/table.rs` exist; Consorsbank gets no entry: learning on `9075` covers it, confirmed live.

## HKKAZ v7 with `IBAN:BIC` only: 9010

- `HKKAZ:3:7+<IBAN>:<BIC>+N+<from>+<to>` with `HKTAN:4:7+4+HKKAZ` gets `9050` + `9010 Verarbeitung nicht möglich` (+ `3076`) for every account. HKSAL v6 (national account) with HKTAN works.
- HISPAS says national account details are allowed: `HISPAS:8:1:4+1+1+0+J:J:J:...` (second flag, "nationale Kontoverbindung erlaubt").
- **What the client sends (confirmed live 2026-11-16: HKKAZ 0020 or 3010 on every account, no 9010):** when the BPD's HISPAS allows it and the account has a number and BLZ, KTI is sent in full (`IBAN:BIC:number:subaccount:280:BLZ`) in HKSAL v7/8, HKKAZ v7 and HKCAZ. Without the flag, or without national data, it stays `IBAN:BIC`. This is driven by the BPD, not by a Consorsbank profile entry.

## Account list and data

- UPD version 0 (dialog-only, Formals): the client does not store the UPD; every dialog receives it again (live 2026-11-16).
- The depot has no IBAN; its id is the account number. Its HIUPD (v6) has no Kontoart and lists only HKWPD and HKPRO as allowed orders. HKSAL for it gets `9380 Keine Berechtigung für diese Auftragsart mit diesem Konto`.
- **What the client does (confirmed live 2026-11-16: the depot got HKWPD only, without HKTAN, answered 0020 with an empty MT535 `:17B::ACTI//N`; no HKSAL/HKKAZ sent):** the per-account run follows the UPD's allowed orders: HKSAL, HKKAZ and HKWPD are only sent when the account lists them (a UPD that lists none restricts nothing), and an account without Kontoart counts as a depot when it lists HKWPD and none of HKSAL, HKKAZ, HKCAZ (`AccountInfo::class`, source `AllowedJobs`); with other jobs listed it stays unclassified, not checking. `Capabilities` shows the allowance per job (`JobCapability::upd`) and the library's jobs never block on it; the caller decides, as `testbank::replay::standard_run` does, which reports skipped orders in `RunReport::skipped`, not as a bank error.
- **Storage period 90 days** (HIKAZS v3 to v7, live trace). The library raises an older `from` to today minus 90 days and reports `ClientWarning::RangeClamped`, so a caller syncing incrementally must sync at least every 90 days. A user report (2026-11, **unverified**) says a longer request is answered with warning `3997` (bank-defined range 3900–3999) and only 90 days of data; the clamp avoids it, and a `3997` would reach the caller as a bank notice.
- No HKCAZ: transactions come as MT940 (HKKAZ v7). **Consorsbank sends no pending bookings over FinTS** (live 2026-11-19): on a day when the bank's own app listed card payments as "Vorgemerkt", all six HKKAZ:7 answers carried the MT940 and no MT942 part (`Pending::NotSent`, not an empty `Reported`); the trace replays clean. The bank's app describes "Vorgemerkt" as submitted SEPA direct debits and authorised card payments; neither reaches a FinTS client.
- Live 2026-11-19: 7 accounts, 117 bookings since 2026-10-01 and six balances were read; 20 bookings carry the counterparty IBAN and BIC. No account carries a Kontoart; the product names are the only hint at the account type.
- The BPD carries two segments of the bank's own, `XIADAS:1` and `XIADSS:1`, each with the body `1+1` (live 2026-11-20). Their code has the form of a job parameter segment (Formals D.6: `I` in second place, `S` at the end) with a first letter of the bank's own; fints-client treats them as bank parameters, so a trace keeps them readable.
- HKWPD answers without error and returns no positions for an empty depot (confirmed in the bank's own app). An empty MT535 is a valid answer, not an error.
- Per a banking app's help page, Consorsbank's FinTS supports SEPA transfers only: no scheduled payments, standing orders or instant payments (**unverified**). The BPD contradicts it for instant payments: it lists HKIPZ 1 (also in a 2026-11 BPD read by someone else); standing orders and HKEKA/HKEKP/HKKAU are absent in both.

## Transaction format (MT940)

- Business transaction codes are the legacy ones (005, 016, 066, …), not the SEPA 1xx range, so a parser that goes by the code treats every booking as non-SEPA.
- The counterparty IBAN and BIC therefore arrive in the national account number and bank code fields (internal bookings carry a real account number and bank code there).
- The SEPA references stay in the purpose, which always starts `EREF+…KREF+…SVWZ+…`. The purpose lines are 27-character fragments that split words: join them without a separator, then split by tag. `EREF+NOTPROVIDED`, an empty `EREF+` and `KREF+NONREF` mean "no reference".
- A running balance comes with each booking. The balance response has booked, available and credit line, but no pending balance.

## For the bank profile

- Offline model: `testbank/src/profiles.rs` (`consorsbank()`); evidence: `testbank/fixtures/replay/consorsbank-76030080.trace.json`; behaviour tests in `tests/bank_profiles_test.rs`.
- Generic: the library retries with HKTAN on `9075` and learns it (`src/protocol/tan.rs`). HIPINS is not reliable.
- Profile entry (`src/banks/table.rs`, 76030080 and 70120400): `user_id` rule, `001` completion for 9 or 10 digits, applied by the library. No TAN override: `9075` learning covers HIPINS. HKKAZ KTI and the per-account orders are generic (BPD and UPD driven).
- No booking ID: transactions come as MT940, and the `:61:` bank reference is never one, so `booking_id` is `None`.
- SEPA tags in the purpose, with `NOTPROVIDED`/`NONREF` as absent: fints-client reads them in `formats` for every bank: `purpose` is the `SVWZ+` value, `end_to_end_id` and `customer_reference` are `None` for the placeholders, the value as sent is in `raw["sepa"]`.

## Payments (live 2026-11-18, 1.00 EUR test)

- Seven accounts, one of them a depot. The login gets `3076` on HKIDN.
- **Transfer (HKCCS):** `HKCCS:1 + HKTAN(4, HKCCS)` is answered `0030` and `3955` **on HKCCS**, not on HKTAN (the TAN codes are reported against the order segment). There is **no payee check at all**: no HKVPP is sent, no 3090, 3945 or 3092. The decoupled status request HKTAN(S) gets `0010`. Receipt status `Accepted`. HKEND is answered `0020`. The library reads the challenge from the HITAN, wherever the codes sit.
- **Not seen yet:** how a transfer to a payee with a wrong name ends without a payee check (3092 or 9076 would be recorded here), an instant payment.
- The BPD has no HIVPPS: no payee check is sent. The EU payee check (Regulation (EU) 2024/886) applies to euro-area PSPs from 2025-10-09, so the bank may offer it in its app only; over FinTS the generic flow without HIVPPS applies (`3092` or `9076`, payments design 3.6). Open.
- HKCCS and HKIPZ 1 list six pain.001 descriptors (001.001.09, .001.03, .003.03, .002.03 and two older ones). HIPINS: HKCCS and HKIPZ need a TAN.
- **No HKIPS** (no HIIPSS): after an instant payment answered `3045` the bank offers no status request. `instant_status` then reads the account's statement from the receipt's `sent_on` and looks for a debit with the payment's end-to-end ID (`StatusSource::Statement`): booked is `Executed`, not found is `None`. Consorsbank sends no pending bookings, so the payment shows only once booked. Generic rule, no profile entry; test `instant_status_without_hkips_reads_the_statement` in `tests/payments_test.rs`.
- Offline model: `profiles::consorsbank()` (`tan_codes_on_order`); test `consorsbank_tan_codes_on_the_order_are_a_challenge` in `tests/payments_test.rs`.
