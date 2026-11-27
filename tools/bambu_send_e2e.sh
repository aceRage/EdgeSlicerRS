#!/usr/bin/env bash
# The send to a Bambu Lab account's printer, end to end, on 127.0.0.1 only: the desktop app's
# own code (apps/desktop/src/api/bambu_send_e2e.rs) drives the real ultranet-bridge against
# UltraNet's tools/fakecloud.py (the account) and tools/fakeprinter.py (the printer). It signs
# in, offers the account's printer in "Send print job to", enters 127.0.0.1 as its address,
# connects and sends with the code the account gave, and the plate prints; then a wrong code, a
# printer whose FTPS cannot be reached and a refused upload each give their own message.
# Last, everything the bridge wrote to standard error is searched for every secret (the test
# itself searches its data folder, where the app, the bridge and UltraNet write).
#
#   tools/bambu_send_e2e.sh <ultranet-bridge> <UltraNet's tools folder>
#
# Nothing here contacts a printer, the LAN or a cloud: the bridge's cloud calls go to the fake
# (ULTRANET_TEST_CLOUD_ENDPOINT, loopback only), its FTPS to a high loopback port
# (ULTRANET_TEST_FTPS_PORT, loopback only), MQTT to the fake on 127.0.0.1:8883. The codes are
# kept in memory, never in the keychain. Prints PASS/FAIL per case and E2E_EXIT=<n> last.
set -u
BRIDGE=$(cd "$(dirname "${1:?usage: bambu_send_e2e.sh <ultranet-bridge> <UltraNet tools>}")" && pwd)/$(basename "$1")
TOOLS=$(cd "${2:?usage: bambu_send_e2e.sh <ultranet-bridge> <UltraNet tools>}" && pwd)
HERE=$(cd "$(dirname "$0")/.." && pwd)
PY=${PYTHON:-python3}
SERIAL=01P09C470100142
CODE=35792468          # the account's code for SERIAL (fakecloud.py)
FTP_ONLY=11223344      # the wrong-code case's printer wants this for FTP
FTPS_PORT=${FTPS_PORT:-19990}
WORK=$(mktemp -d "${TMPDIR:-/tmp}/bambu_send_e2e.XXXXXX")
OUTSIDE="$WORK/outside"   # the fakes' own files: they hold the codes, and are not searched
mkdir -p "$OUTSIDE" "$WORK/bin" "$WORK/stderr"
FAILS=0
FAKE_PID=
CLOUD_PID=
say()  { printf '%s\n' "$*"; }
pass() { say "PASS  $*"; }
fail() { say "FAIL  $*"; FAILS=$((FAILS + 1)); }
stop_fake()  { [ -n "$FAKE_PID" ] && { kill "$FAKE_PID" 2>/dev/null; wait "$FAKE_PID" 2>/dev/null; FAKE_PID=; }; }
stop_cloud() { [ -n "$CLOUD_PID" ] && { kill "$CLOUD_PID" 2>/dev/null; wait "$CLOUD_PID" 2>/dev/null; CLOUD_PID=; }; }
trap 'stop_fake; stop_cloud; rm -rf "$WORK"' EXIT

[ -x "$BRIDGE" ] || { say "no bridge at $BRIDGE"; say "E2E_EXIT=1"; exit 1; }
for port in 8883 "$FTPS_PORT"; do
    if lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
        say "port $port is in use on this machine; stop what holds it first"; say "E2E_EXIT=1"; exit 1
    fi
done

CLOUD_PORT=$(( 40000 + RANDOM % 20000 ))
while lsof -nP -iTCP:"$CLOUD_PORT" -sTCP:LISTEN >/dev/null 2>&1; do CLOUD_PORT=$(( 40000 + RANDOM % 20000 )); done
mkdir -p "$OUTSIDE/cloud_certs"
"$PY" -u "$TOOLS/fakecloud.py" --port "$CLOUD_PORT" --certs "$OUTSIDE/cloud_certs" --out "$OUTSIDE/cloud_requests.jsonl" \
      --seconds 3600 > "$OUTSIDE/fakecloud.log" 2>&1 &
CLOUD_PID=$!
for _ in $(seq 1 600); do
    grep -q '^READY' "$OUTSIDE/fakecloud.log" 2>/dev/null && break
    kill -0 "$CLOUD_PID" 2>/dev/null || break
    sleep 0.1
done
grep -q '^READY' "$OUTSIDE/fakecloud.log" || { say "fakecloud.py did not start"; tail -20 "$OUTSIDE/fakecloud.log"; say "E2E_EXIT=1"; exit 1; }
export ULTRANET_TEST_CLOUD_ENDPOINT="localhost:$CLOUD_PORT" SSL_CERT_FILE="$OUTSIDE/cloud_certs/ca.pem"

# The app starts the bridge as its child and throws its stderr away: a wrapper keeps it.
cat > "$WORK/bin/ultranet-bridge" <<EOF
#!/bin/sh
cd "$(dirname "$BRIDGE")" || exit 1
exec "$BRIDGE" "\$@" 2>>"$WORK/stderr/bridge_\${EDGE_E2E_SEND:-none}.log"
EOF
chmod +x "$WORK/bin/ultranet-bridge"

# start_fake <case> [fakeprinter args...]: MQTT 8883 and FTPS FTPS_PORT, until both listen.
start_fake() {
    local name=$1; shift
    local log="$OUTSIDE/fake_$name.log"
    mkdir -p "$OUTSIDE/disk_$name"
    "$PY" -u "$TOOLS/fakeprinter.py" --out "$OUTSIDE/rec_$name.json" --seconds 900 --ftp-port "$FTPS_PORT" \
          --store-dir "$OUTSIDE/disk_$name" --serial "$SERIAL" --model C12 --name "Workshop P1S" \
          --access-code "$CODE" --idle --report-only "$@" > "$log" 2>&1 &
    FAKE_PID=$!
    for _ in $(seq 1 100); do
        if [ "$(grep -o 'listening on ' "$log" 2>/dev/null | wc -l)" -ge 2 ]; then return 0; fi
        sleep 0.1
    done
    say "fake printer ($name) did not come up:"; tail -20 "$log"
    return 1
}

# run_case <case> <bridge's FTPS test port>: the test, its verdict.
run_case() {
    local name=$1 port=$2
    local out="$OUTSIDE/test_$name.log"
    ( cd "$HERE/apps/desktop" &&
      EDGE_ULTRANET_BRIDGE="$WORK/bin/ultranet-bridge" EDGE_E2E_SEND="$name" ULTRANET_TEST_FTPS_PORT="$port" \
      EDGE_E2E_RECORD="$OUTSIDE/rec_$name.json" EDGE_E2E_STORE="$OUTSIDE/disk_$name" EDGE_SECRETS=memory \
      cargo test --quiet an_account_printer_gets_the_plate_over_the_lan_through_the_real_bridge -- --nocapture \
    ) > "$out" 2>&1
    if grep -q '^test result: ok. 1 passed' "$out"; then
        pass "[$name] $(grep -E "^e2e $name: (\{|[0-9])" "$out" | tail -1 | cut -c1-240)"
        grep -E '^e2e print: .*(md5|byte|equals)' "$out" | sed 's/^/      /'
    else
        fail "[$name] the test did not pass:"; grep -E -A4 'panicked' "$out" | head -15
    fi
}

if start_fake print; then run_case print "$FTPS_PORT"; else fail "fake printer (print)"; fi
stop_fake
if start_fake wrong_code --ftp-access-code "$FTP_ONLY"; then run_case wrong_code "$FTPS_PORT"; else fail "fake printer (wrong_code)"; fi
stop_fake
# A loopback port nothing listens on (bound, then let go), checked closed.
CLOSED=$("$PY" -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')
if lsof -nP -iTCP:"$CLOSED" -sTCP:LISTEN >/dev/null 2>&1; then
    fail "port $CLOSED is not closed"
elif start_fake unreachable; then
    run_case unreachable "$CLOSED"
else
    fail "fake printer (unreachable)"
fi
stop_fake
if start_fake upload --reject-stor-named verify_job; then run_case upload "$FTPS_PORT"; else fail "fake printer (upload)"; fi
stop_fake

# The bridge's stderr: kept for every case, and no secret in it.
SECRETS="-e $CODE -e $FTP_ONLY -e B4K9Q2ZP -e GoodT1 -e Fail01 -e AT-fake- -e RT-fake-"
mkdir -p "$OUTSIDE/control"; printf 'x%sx\n' "$CODE" > "$OUTSIDE/control/planted"
if [ "$(grep -rlaF $SECRETS "$OUTSIDE/control" | wc -l | tr -d ' ')" -ne 1 ]; then
    fail "the search does not find a planted code: the check below would mean nothing"
fi
LOGS=$(ls "$WORK"/stderr/bridge_*.log 2>/dev/null | wc -l | tr -d ' ')
FOUND=$(grep -rlaF $SECRETS "$WORK/stderr" "$WORK/bin" 2>/dev/null)
if [ "$LOGS" -eq 4 ] && [ -z "$FOUND" ]; then
    pass "no access code, ticket or token on the bridge's stderr ($LOGS cases, $(cat "$WORK"/stderr/*.log | wc -l | tr -d ' ') lines)"
else
    fail "the bridge's stderr: $LOGS of 4 kept; a secret in: ${FOUND:-none}"
fi
say "E2E_EXIT=$FAILS"
exit "$FAILS"
