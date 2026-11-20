---
name: release
description: Cut a fints-client release: changelog, version, tag, GitHub release with a marketing description, crates.io.
disable-model-invocation: true
---

# Release fints-client

A release is one version number in four places that must agree: the three `Cargo.toml` files, the `CHANGELOG.md` heading, the git tag `vX.Y.Z` and the GitHub release. The argument is the version (`/release 0.1.0`); without one, propose it in step 2.

Steps 1 to 6 change only local files. Step 7 is the first that leaves the machine, and it runs only after the human approves the release description.

## 1. Check the starting state

- The branch is `main`, the working tree is clean, and `main` equals `origin/main`.
- The previous release is the newest `v*` tag (`git describe --tags --abbrev=0`); with no tag, the range is the whole history.

Done when both hold. If one fails, report it and stop.

## 2. Make the changelog match the commits

Read every commit since the previous release (`git log --no-merges <tag>..HEAD`) and the diff of the public surface (`git diff <tag>..HEAD -- src/lib.rs src/jobs.rs src/jobs/ src/error.rs src/types.rs Cargo.toml`). Compare with the `Unreleased` section of `CHANGELOG.md`:

- A change a user of the library would notice (API, behaviour, a bank, a cargo feature, a file format, a script) needs an entry. Add the missing ones in the changelog's style.
- An entry needs a commit in the range that did it. Remove or correct the ones that have none.
- Internal changes (refactors, tests, formatting, docs wording) get no entry.

Then settle the version. Before 1.0.0: a removed or changed public item, trace format or state format raises the minor number; anything else raises the patch number. Show the human the list of added and removed entries and the version, and wait for their agreement.

Done when every user-visible commit maps to an entry, every entry maps to a commit, and the human agreed to the version.

## 3. Set the version

- `version` in `Cargo.toml`, `derive/Cargo.toml` and `testbank/Cargo.toml`.
- The `version` of the `fints-client-derive` dependency in `Cargo.toml` and of the `fints-client` dependency in `testbank/Cargo.toml`.
- Run `cargo check --workspace` so `Cargo.lock` follows.
- In `CHANGELOG.md`, rename `## Unreleased` to `## X.Y.Z — YYYY-MM-DD` (today) and put a new empty `## Unreleased` above it.

Done when `git grep -n 'version = "' -- '*Cargo.toml'` shows the new number for all three crates and both internal dependencies.

## 4. Run the checks

Run the checks of the "Done" section in `AGENTS.md`, then `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --features http,trace -p fints-client` (the features docs.rs builds with) and `cargo package --list -p fints-client`. The package list must hold no file under `testbank/` or `docs/`, and under `tests/` only `tests/fixtures/pain/` (golden files the unit tests read). Also run the packaged unit tests: `cargo package -p fints-client-derive -p fints-client`, then in `target/package/fints-client-<version>` run `cargo test --lib --config 'patch.crates-io.fints-client-derive.path="../../../derive"'`.

Done when every command passes. A failure is fixed in its own commit before the release continues.

## 5. Write the release description

Write `target/release-notes-X.Y.Z.md` in two parts.

**The pitch** comes first: a headline and two to four sentences that sell the release to a Rust developer who needs German online banking. Lead with what they can do now that they could not do before. Name the concrete thing (a bank, an operation, a type), and give a breaking change one plain sentence on what to change. Every claim traces to an entry of this version's changelog section or to the tested-banks table in `README.md`.

**The changes** follow under `## Changes`: this version's section of `CHANGELOG.md`, copied verbatim without its version heading.

Example of the pitch:

```markdown
## Transfers with the payee check, at three banks

fints-client 0.1.0 is the first release: read accounts, balances, transactions and holdings, and send SEPA transfers with Verification of Payee, tested live at DKB, comdirect and Consorsbank. Every operation returns a step you resume, so an app approval never blocks your code.
```

Done when each sentence of the pitch points at a changelog entry or a row of the tested-banks table, and the part under `## Changes` is identical to the changelog section.

## 6. Commit and get approval

Commit the version and changelog changes as `release X.Y.Z`. Show the human the release description and the list of what step 7 will do, and wait for an explicit yes.

Done when the human approved the description as shown. A change to it means showing it again.

## 7. Publish

In this order, stopping at the first failure:

1. `git tag -a vX.Y.Z -m "fints-client X.Y.Z"`, then `git push origin main vX.Y.Z`.
2. `gh release create vX.Y.Z --title "fints-client X.Y.Z" --notes-file target/release-notes-X.Y.Z.md`.
3. crates.io, only when the human asked for it in this session: the `publish = false` lines in `derive/Cargo.toml` and `Cargo.toml` must already be gone (removing them is the human's decision, in its own commit before step 1). Then `cargo publish -p fints-client-derive`, wait until the index lists the version, then `cargo publish -p fints-client`. `fints-client-testbank` is never published.

A published crate version cannot be replaced, only yanked. If step 3 fails after the derive crate went out, fix forward with a new patch version.

## 8. Verify

- `gh release view vX.Y.Z --json body -q .body` ends with the same text as this version's section of `CHANGELOG.md` at the tag (`git show vX.Y.Z:CHANGELOG.md`).
- The tag, the changelog heading and `cargo metadata --no-deps` agree on the version.
- If published: `cargo search fints-client` shows the version.

Report the release URL and each check's result.
