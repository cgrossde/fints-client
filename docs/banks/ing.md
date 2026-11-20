# ING

Part of the [bank list](../banks.md). Anything not confirmed by a live run or the bank itself is marked **unverified**.

Bank code 50010517, server from the bank directory.

Source: not yet observed by a fints-client run (**unverified**).

- **Risk (spec):** the spec has PIN/TAN clients obtain a system ID by synchronisation (Formals C.8, sync with `0`) and put HKTAN in the dialog init when the product supports SCA (PIN/TAN B.4). The direct login does neither: ING may answer 9210 or 9800 for an unknown system ID, or 9075 for the missing HKTAN. No working run is known.
- **Live run 1:** `scripts/try-bank.sh 50010517` with the profile as it is. Check the first message's HIRMG/HIRMS (0020, 3920, 3060 against 9xxx), whether HIBPA, HITANS, HIUPD and a system ID arrive, and whether HKSAL and HKKAZ work without HKTAN.
- **If run 1 fails with 9xxx:** delete the ING entry in `src/banks/table.rs`, so ING gets the standard flow (sync, then init with HKTAN), and run `scripts/try-bank.sh 50010517` again. If that works, remove `InitMode::Direct`.

- The business dialog opens without a synchronisation dialog: a stored or client-generated system ID (16 hex digits), security function 900, PIN only, no HKTAN. The bank sends its parameters in that dialog.
- No booking ID (nothing known about its references): `booking_id` is `None`.
- Profile: `BankProfile.init = InitMode::Direct { security_function: "900" }`, set for 50010517 in `src/banks/table.rs`; `jobs::open` honours it, also under `FINTS_URL` and in replays (`BankConfig::with_url` keeps the profile). Test: `direct_init_sends_no_sync` (`tests/jobs_test.rs`).
