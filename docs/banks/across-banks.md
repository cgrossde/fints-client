# Across banks

Part of the [bank list](../banks.md): what several banks have in common, and banks without a page of their own.

## Patterns

- **The bank's parameter data is not a complete contract.** Missing BICs (DKB), missing account types (comdirect) and a TAN table that contradicts the server's behaviour (Consorsbank) all needed workarounds.
- **An account's identity at a bank is not its IBAN.** comdirect gives two accounts the same IBAN; DKB's credit card has none.
- **The FinTS user ID is not always the web login.** Consorsbank appends a 3-digit user number; the library completes the short form (profile `user_id` rule).
- **When a bank asks for app approval differs per bank and per request.** DKB asks at login, comdirect and Consorsbank (so far) not at all.

## Savings banks (reported, unverified here)

Behaviour reported for a savings bank (Finanz-Informatik platform) by code fints-client builds on (see `NOTICE`), taken in as generic behaviour, none of it run by us:

- A continuation request (touchdown point) for HKKAZ without HKTAN:4 was rejected with `9370` (signatures insufficient) although the first page had HKTAN. fints-client sends HKTAN:4 with every page when HIPINS asks for a TAN; other banks are expected to answer `3076`.
- The bank answers a rejected order with a global `9050` around the specific segment code; fints-client reports the segment code.
- Depots appear in the UPD without IBAN and with account type 30.

No profile exists for the savings banks (Sparkasse / Finanz-Informatik). Their proprietary jobs (`DKKKU`/`DIKKUS` credit-card bookings, `DKWDH`/`DIWDHS` depot history) have no public specification. Official stand-in for card bookings is **HKKKU** (CR0538 on fints.org). comdirect advertises `DIKKUS` in the BPD. fints-client does not send these orders.
