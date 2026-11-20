#!/usr/bin/env bash
# Tries fints-client at a bank it has not been tested with, and records the
# dialog as a masked trace. A human runs this; it contacts the bank.
# The walk-through is docs/adding-a-bank.md.
#
#   scripts/try-bank.sh <BLZ> [days]      test the bank with this bank code
#   scripts/try-bank.sh <BLZ> --transfer  send ONE REAL TRANSFER and record it
#   scripts/try-bank.sh <words>           search the bank directory by name or city
#   scripts/try-bank.sh --dry-run         the test run against the local test bank
#
# What a test run does: logs in (you confirm in your banking app or type a
# TAN if the bank asks), reads balance and transactions of every account and
# the holdings of a depot, and ends the dialog. It only reads; it sends no
# order. Then it replays the recording offline, as the test suite would.
#
# The BLZ is characters 5 to 12 of a German IBAN. days (default 30) is how
# far back transactions are read.
#
# --transfer MOVES REAL MONEY (examples/transfer.rs): it is the only way to
# record a transfer and the payee check. It asks for the payee name, IBAN,
# BIC (optional), amount (default 1.00), purpose and instant choice (no,
# prefer or require; default no) with visible prompts, never takes them from
# the environment, then asks which of your accounts to debit and, if the bank
# wants it, to confirm the payee check. An amount above 1.00 is refused
# unless FINTS_ALLOW_AMOUNT is set. Use your own account at another bank as
# the payee.
#
# User ID and PIN are always asked for with hidden input, so they never reach
# the command line or the shell history. The product ID comes from
# FINTS_PRODUCT_ID, from a FINTS_PRODUCT_ID= line in .env (git ignores it;
# see .env.example) or a hidden prompt. FINTS_TRACE_SALT (64 hex digits) is
# read the same way; with it, traces of several runs share their reference
# pseudonyms. FINTS_URL overrides the URL of the bank directory. The values
# live only in this script's environment and are cleared on exit.
#
# If the bank sends a segment fints-client does not know, the run shows it
# and asks whether the trace may keep it readable.
#
# Result: testbank/fixtures/replay/local/<bank>-<BLZ>.trace.json (for example
# dkb-12030000.trace.json; <bank> is a short name taken from the directory), or
# <bank>-<BLZ>.transfer.trace.json for a transfer; git ignores the folder. A failed
# run is kept beside it as <bank>-<BLZ>[.transfer].failed-<time>.json, a
# transfer you cancelled as <bank>-<BLZ>.transfer.cancelled-<time>.json; the previous
# trace stays. The replay tests skip failed runs and transfer traces.

set -euo pipefail

usage() {
    sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
}

cd "$(dirname "$0")/.."
directory=data/fints_institutes.csv

# The directory is ISO 8859-1 with ; as separator: BLZ in column 2, name in
# 4, city in 5, PIN/TAN URL in 25.
directory_rows() {
    iconv -f ISO-8859-1 -t UTF-8 "$directory" | tr -d '\r'
}

dry_run() {
    local port=47331 out=target/try-bank/dry-run.trace.json
    if (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
        echo "Port $port is in use; stop the other fints-testbank first." >&2
        exit 1
    fi
    echo "Building the test bank and the recorder ..." >&2
    cargo build -q -p fints-client-testbank --bin fints-testbank
    cargo build -q --features trace --example record_trace
    mkdir -p "$(dirname "$out")"

    target/debug/fints-testbank --port "$port" >/dev/null 2>&1 &
    bank_pid=$!
    trap 'kill "${bank_pid:-}" 2>/dev/null || true' EXIT
    for _ in $(seq 1 50); do
        (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null && break
        sleep 0.1
    done

    echo "Recording a dialog with the local test bank (synthetic data, no bank contact)" >&2
    FINTS_URL="http://127.0.0.1:$port/fints" FINTS_BLZ=12345678 FINTS_USER=testuser1 \
        FINTS_PIN=test-pin-1 FINTS_PRODUCT_ID=TEST-123 \
        cargo run -q --features trace --example record_trace -- "$out" 30
    echo >&2
    echo "Dry run done. The trace is $out" >&2
    echo "A real bank works the same way: scripts/try-bank.sh <BLZ>" >&2
}

ask() {
    local name=$1 prompt=$2 value
    read -r -s -p "$prompt: " value
    echo >&2
    [[ -n $value ]] || { echo "$name is empty" >&2; exit 1; }
    printf -v "$name" '%s' "$value"
    export "${name?}"
}

# Reads one line of .env; the file is not sourced.
from_dotenv() {
    local name=$1 value
    [[ -z ${!name:-} && -f .env ]] || return 0
    value=$(sed -n "s/^$name=//p" .env | head -n 1)
    value=${value//[\"\']/}
    [[ -n $value ]] || return 0
    printf -v "$name" '%s' "$value"
    export "${name?}"
}

# A short name of the bank for file names, from its URL $1 and its directory
# name $2: the host's domain name without its ending (fints.dkb.de gives
# dkb) when the bank's name contains it, else the name itself, cut at 30
# characters. Many banks share a data centre, whose host says nothing.
bank_slug() {
    local host label name
    host=${1#*://}
    host=${host%%[:/]*}
    label=$(echo "$host" | awk -F. '{ print tolower($(NF > 1 ? NF - 1 : NF)) }')
    name=$(echo "$2" | tr '[:upper:]' '[:lower:]' |
        sed -e 's/ä/ae/g; s/ö/oe/g; s/ü/ue/g; s/ß/ss/g; s/Ä/ae/g; s/Ö/oe/g; s/Ü/ue/g' \
            -e 's/[^a-z0-9]\{1,\}/-/g; s/^-//; s/-$//')
    if [[ -n $label && $name == *"$label"* ]]; then
        echo "$label"
    else
        name=${name:0:30}
        echo "${name%-}"
    fi
}

# Visible prompt with an optional default; an empty answer takes the default
# (which may be empty).
ask_visible() {
    local name=$1 prompt=$2 default=${3:-} value
    if [[ -n $default ]]; then
        read -r -p "$prompt [$default]: " value
    else
        read -r -p "$prompt: " value
    fi
    printf -v "$name" '%s' "${value:-$default}"
    export "${name?}"
}

secrets='FINTS_USER FINTS_PIN FINTS_PRODUCT_ID FINTS_TRACE_SALT FINTS_PAYEE_NAME FINTS_PAYEE_IBAN FINTS_PAYEE_BIC FINTS_AMOUNT FINTS_PURPOSE FINTS_INSTANT'
log=
# shellcheck disable=SC2086
cleanup() {
    unset $secrets
    [[ -z $log ]] || rm -f "$log"
}

ask_credentials() {
    local blz=$1
    ask FINTS_USER "User ID for BLZ $blz"
    ask FINTS_PIN "PIN"
    from_dotenv FINTS_PRODUCT_ID
    from_dotenv FINTS_TRACE_SALT
    [[ -n ${FINTS_PRODUCT_ID:-} ]] || ask FINTS_PRODUCT_ID "Registered product ID"
    export FINTS_BLZ=$blz
}

# The payee is data of this run only: ask, never take from the environment.
ask_order() {
    local whole frac cents
    unset FINTS_PAYEE_NAME FINTS_PAYEE_IBAN FINTS_PAYEE_BIC FINTS_AMOUNT FINTS_PURPOSE FINTS_INSTANT
    ask_visible FINTS_PAYEE_NAME "Payee name"
    [[ -n $FINTS_PAYEE_NAME ]] || { echo "payee name is empty" >&2; exit 1; }
    ask_visible FINTS_PAYEE_IBAN "Payee IBAN"
    FINTS_PAYEE_IBAN=${FINTS_PAYEE_IBAN// /}
    [[ $FINTS_PAYEE_IBAN =~ ^[A-Za-z]{2}[0-9]{2}[A-Za-z0-9]{11,30}$ ]] || { echo "that is not an IBAN" >&2; exit 1; }
    ask_visible FINTS_PAYEE_BIC "Payee BIC (optional)"
    ask_visible FINTS_AMOUNT "Amount in EUR" "1.00"
    [[ $FINTS_AMOUNT =~ ^[0-9]+(\.[0-9]{1,2})?$ ]] || { echo "amount must look like 1.00 (digits, a point, at most two decimals)" >&2; exit 1; }
    # Compare in cents: no floating point, no locale.
    whole=${FINTS_AMOUNT%%.*}
    frac=${FINTS_AMOUNT#*.}
    [[ $FINTS_AMOUNT == *.* ]] || frac=00
    frac=$(printf '%-2s' "$frac")
    frac=${frac// /0}
    cents=$((10#$whole * 100 + 10#$frac))
    if ((cents == 0)); then
        echo "amount must be greater than zero" >&2
        exit 1
    fi
    if ((cents > 100)) && [[ -z ${FINTS_ALLOW_AMOUNT:-} ]]; then
        echo "amount $FINTS_AMOUNT is above 1.00; set FINTS_ALLOW_AMOUNT to send it" >&2
        exit 1
    fi
    ask_visible FINTS_PURPOSE "Purpose" "fints-client test"
    ask_visible FINTS_INSTANT "Instant transfer (no, prefer, require)" "no"
    [[ $FINTS_INSTANT =~ ^(no|prefer|require)$ ]] || { echo "instant must be no, prefer or require" >&2; exit 1; }
}

# Runs example $1 with the trace path and the arguments after $2, and keeps
# the result as $2.trace.json, as $2.cancelled-<time>.json when the person
# stopped a transfer, or as $2.failed-<time>.json when the run
# failed. It records to a scratch name first, so a failed run never replaces
# a good trace.
run_example() {
    local example=$1 stem=$2 stamp tmp
    shift 2
    stamp=$(date +%Y%m%d-%H%M%S)
    tmp="$stem.recording-$stamp.json"
    log=$(mktemp)
    cargo run -q --features trace --example "$example" -- "$tmp" "$@" | tee "$log"
    [[ -f $tmp ]] || return 1
    if grep -q "^The run was cancelled" "$log"; then
        mv "$tmp" "$stem.cancelled-$stamp.json"
        echo "Cancelled: kept as $stem.cancelled-$stamp.json; $stem.trace.json is unchanged." >&2
        return 0
    fi
    if grep -q "^The run failed" "$log"; then
        mv "$tmp" "$stem.failed-$stamp.json"
        echo "Run failed: kept as $stem.failed-$stamp.json; $stem.trace.json is unchanged." >&2
        return 1
    fi
    mv "$tmp" "$stem.trace.json"
    echo "Trace saved to $stem.trace.json" >&2
}

transfer=
args=()
for arg in "$@"; do
    if [[ $arg == --transfer ]]; then transfer=1; else args+=("$arg"); fi
done
set -- ${args[@]+"${args[@]}"}

[[ $# -ge 1 ]] || usage
case $1 in
    -h | --help) usage ;;
    --dry-run)
        [[ -z $transfer ]] || usage
        dry_run
        exit 0
        ;;
esac

if [[ ! $1 =~ ^[0-9]{8}$ ]]; then
    [[ -z $transfer ]] || usage
    # Not a bank code: search the directory and stop.
    query=$*
    matches=$(directory_rows | awk -F';' -v q="$query" '
        BEGIN { q = tolower(q) }
        NR > 1 && $25 != "" && index(tolower($4 " " $5), q) { printf "  %s  %s, %s\n", $2, $4, $5 }
    ' | sort -u)
    if [[ -z $matches ]]; then
        echo "No bank with a PIN/TAN address matches \"$query\"." >&2
        echo "Take the BLZ from your IBAN (characters 5 to 12) instead." >&2
        exit 1
    fi
    echo "Banks matching \"$query\" (BLZ, name, city):" >&2
    echo "$matches" | head -n 40
    count=$(echo "$matches" | wc -l | tr -d ' ')
    [[ $count -le 40 ]] || echo "  ... and $((count - 40)) more; narrow the search." >&2
    echo >&2
    echo "Run again with the BLZ from your IBAN: scripts/try-bank.sh <BLZ>" >&2
    exit 0
fi

blz=$1
days=${2:-30}
[[ $days =~ ^[0-9]+$ && $# -le 2 ]] || usage
[[ -z $transfer || $# -eq 1 ]] || usage
local_dir=testbank/fixtures/replay/local
slug=

row=$(directory_rows | awk -F';' -v b="$blz" 'NR > 1 && $2 == b && $25 != "" && !seen { print $4 ";" $5 ";" $25; seen = 1 }')
if [[ -n $row ]]; then
    IFS=';' read -r name city url <<<"$row"
    slug=$(bank_slug "${FINTS_URL:-$url}" "$name")
    echo "Bank:  $name, $city (BLZ $blz)" >&2
    echo "URL:   ${FINTS_URL:-$url}" >&2
elif [[ -n ${FINTS_URL:-} ]]; then
    echo "Bank:  BLZ $blz, not in the directory" >&2
    echo "URL:   $FINTS_URL" >&2
else
    echo "BLZ $blz has no PIN/TAN address in $directory." >&2
    echo "If you know the bank's FinTS URL: FINTS_URL=https://... scripts/try-bank.sh $blz" >&2
    exit 1
fi

# Files of this run: <bank>-<BLZ>.*, or <BLZ>.* for a bank without a name.
stem="$local_dir/${slug:+$slug-}$blz"
mkdir -p "$local_dir"
trap cleanup EXIT

if [[ -n $transfer ]]; then
    cat >&2 <<EOF

This logs in at the bank and sends ONE REAL SEPA TRANSFER from one of your
accounts: real money moves. Use your own account at another bank as the
payee. You need a FinTS product ID (registration:
https://www.fints.org/de/hersteller/produktregistrierung).

The recording stays on this computer. It is masked before it is written: no
PIN, name, IBAN, amount or purpose reaches the file, yours or the payee's.

EOF
    # Build first, so a compile error shows before any credential is typed.
    cargo build -q --features trace --example transfer
    ask_credentials "$blz"
    ask_order
    echo "Sending $FINTS_AMOUNT EUR from BLZ $blz: REAL MONEY. Press Enter to go on, Ctrl-C to stop." >&2
    read -r
    run_example transfer "$stem.transfer" || exit 1
    echo "Read the HIRMS texts of the trace before you share it: a bank may put a name there." >&2
    exit 0
fi

cat >&2 <<EOF

This logs in at the bank with your online-banking user ID and PIN, reads
balances, the transactions of the last $days days and depot holdings, and ends
the dialog. It sends no order. You need a FinTS product ID (registration:
https://www.fints.org/de/hersteller/produktregistrierung).

The recording stays on this computer. It is masked before it is written: no
PIN, name, IBAN, amount or booking text reaches the file. Several failed
logins can lock an online-banking access, so check the user ID and PIN
before you repeat a run.

EOF
read -r -p "Continue? [y/N] " answer
[[ $answer == [yY]* ]] || exit 0

trace="$stem.trace.json"
# Build first, so a compile error shows before any credential is typed.
cargo build -q --features trace --example record_trace
ask_credentials "$blz"
echo "Recording BLZ $blz; confirm in your banking app when asked" >&2
if ! run_example record_trace "$stem" "$days"; then
    cat >&2 <<EOF

The run did not complete. That is the most useful kind of report: the summary
table above shows where the bank refused (FAILED lines, 9xxx codes), and the
recording of the run is kept as
$stem.failed-<time>.json, masked like any trace.

Please open a bank report with the table, and attach the recording if you can:
  https://github.com/cgrossde/fints-client/issues/new?template=bank-report.yml
What the file holds: docs/adding-a-bank.md, step 4.
EOF
    exit 1
fi

echo >&2
echo "Replaying the recording offline ..." >&2
if REPLAY_LOCAL=1 cargo test -q --test replay_test >/dev/null 2>&1; then
    replay="replays clean"
else
    replay="does NOT replay clean; see: REPLAY_LOCAL=1 cargo test --test replay_test"
fi

cat >&2 <<EOF

Done: the bank works with fints-client for login and reading.
  Trace:   $trace ($replay)
  Summary: cargo run -q --features trace --example trace_summary -- $trace

Next: tell us about the bank. Open a bank report with the summary table and,
if you like, the trace file:
  https://github.com/cgrossde/fints-client/issues/new?template=bank-report.yml

The trace holds the bank's messages: parameters, versions and return codes.
It holds no PIN, TAN, user ID, name, IBAN, account number, counterparty,
booking text or amount. It does show how many bookings of which kind you
have; their dates are moved by a few days. It is plain JSON; have a look
before you attach it (docs/adding-a-bank.md, step 4).
EOF
