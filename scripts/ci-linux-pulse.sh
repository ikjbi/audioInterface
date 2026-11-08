#!/usr/bin/env bash
# Integration test for the Linux backend. Needs a running PulseAudio server
# with the virtual devices from .github/workflows/ci.yml:
#   ci_speakers, ci_headphones (null sinks) and ci_sine_mic (440 Hz sine source).
set -euo pipefail
BIN=${1:-target/debug/audio-interface}

step() { echo; echo "=== $* ==="; }
fail() { echo "FAIL: $*" >&2; exit 1; }

step "list"
$BIN list
$BIN list --json > devices.json
python3 - <<'EOF'
import json
d = json.load(open("devices.json"))
names = {(x["direction"], x["id"]) for x in d}
for want in [("output", "ci_speakers"), ("output", "ci_headphones"), ("input", "ci_sine_mic")]:
    assert want in names, f"missing {want}: {names}"
assert not any(x["id"].endswith(".monitor") for x in d), "monitor sources must be hidden"
print("device list OK:", len(d), "devices")
EOF

step "set-default output"
$BIN set-default output ci_headphones
[ "$(pactl get-default-sink)" = "ci_headphones" ] || fail "default sink is $(pactl get-default-sink)"
$BIN set-default output CI_Speakers   # by description
[ "$(pactl get-default-sink)" = "ci_speakers" ] || fail "default sink is $(pactl get-default-sink)"
$BIN list --json | python3 -c 'import json,sys; d=json.load(sys.stdin); assert [x["id"] for x in d if x["is_default"] and x["direction"]=="output"]==["ci_speakers"]'

step "set-default input"
$BIN set-default input ci_sine_mic
[ "$(pactl get-default-source)" = "ci_sine_mic" ] || fail "default source is $(pactl get-default-source)"

step "volume"
$BIN volume output ci_speakers 40
pactl get-sink-volume ci_speakers | grep -q " 40%" || { pactl get-sink-volume ci_speakers; fail "sink volume not 40%"; }
$BIN volume output ci_speakers | grep -q "40%" || fail "volume readback"
$BIN volume input ci_sine_mic 75
pactl get-source-volume ci_sine_mic | grep -q " 75%" || { pactl get-source-volume ci_sine_mic; fail "source volume not 75%"; }
$BIN volume input ci_sine_mic 100

step "mute"
$BIN mute output ci_speakers on
pactl get-sink-mute ci_speakers | grep -q yes || fail "not muted"
$BIN mute output ci_speakers toggle
pactl get-sink-mute ci_speakers | grep -q no || fail "toggle did not unmute"

step "meter input (sine source)"
out=$($BIN meter ci_sine_mic --seconds 3 | tr '\r' '\n' | grep '^summary:')
echo "$out"
python3 - "$out" <<'EOF'
import re, sys
m = re.search(r"max_db=(-?[\d.]+|-?inf) status=(\w+) blocks=(\d+)", sys.argv[1])
assert m, sys.argv[1]
max_db, status, blocks = float(m.group(1)), m.group(2), int(m.group(3))
assert blocks > 10, f"too few audio blocks: {blocks}"
assert max_db > -20, f"sine source should be loud, got {max_db} dBFS"
assert status == "Ok", status
print("input meter OK")
EOF

step "meter output (via monitor source while a tone plays)"
python3 - <<'EOF' > tone.raw
import math, struct, sys
rate = 48000
frames = rate * 6
data = bytearray()
for i in range(frames):
    s = int(0.5 * 32767 * math.sin(2 * math.pi * 1000 * i / rate))
    data += struct.pack("<hh", s, s)
sys.stdout.buffer.write(data)
EOF
pacat --playback --device=ci_headphones --format=s16le --rate=48000 --channels=2 tone.raw &
PLAYER=$!
sleep 0.5
out=$($BIN meter --direction output ci_headphones --seconds 3 | tr '\r' '\n' | grep '^summary:')
kill $PLAYER 2>/dev/null || true
echo "$out"
python3 - "$out" <<'EOF'
import re, sys
m = re.search(r"max_db=(-?[\d.]+) status=(\w+)", sys.argv[1])
assert m, sys.argv[1]
max_db = float(m.group(1))
# 0.5 amplitude tone is about -6 dBFS
assert -12 < max_db < 0, f"expected about -6 dBFS from the monitor, got {max_db}"
print("output meter OK")
EOF

step "errors"
if $BIN set-default output does-not-exist 2>err.txt; then fail "expected failure"; fi
grep -q "no output device matches" err.txt || { cat err.txt; fail "bad error message"; }

echo; echo "ALL LINUX INTEGRATION TESTS PASSED"
