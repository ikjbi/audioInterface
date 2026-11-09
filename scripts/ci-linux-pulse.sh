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

step "balance"
$BIN volume output ci_speakers 80
$BIN balance ci_speakers -0.5 | tee bal.txt
grep -q "balance -0.50" bal.txt || fail "balance readback"
pactl get-sink-volume ci_speakers | tee vol.txt
grep -q "front-left: .* 80%" vol.txt || fail "left channel should stay at 80%"
grep -q "front-right: .* 40%" vol.txt || fail "right channel should be at 40%"
$BIN volume output ci_speakers 60   # changing volume keeps the balance
pactl get-sink-volume ci_speakers | grep -q "front-right: .* 30%" || { pactl get-sink-volume ci_speakers; fail "balance not kept on volume change"; }
$BIN balance ci_speakers 0

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

step "in-use detection (parecord holds the mic)"
parecord --device=ci_sine_mic /dev/null &
REC=$!
sleep 1.5
$BIN list --json | python3 -c '
import json, sys
d = [x for x in json.load(sys.stdin) if x["id"] == "ci_sine_mic"][0]
print("in_use_by:", d["in_use_by"])
assert any("parecord" in a.lower() for a in d["in_use_by"]), d
'

step "auto-record while parecord is using the mic"
rm -rf rec_auto
$BIN record --auto --app parecord --seconds 4 --out rec_auto | tr '\r' '\n' | grep -E "Verdict|Files" || true
kill $REC 2>/dev/null || true
ls rec_auto
ls rec_auto/*.csv >/dev/null || fail "auto-record wrote no CSV"
head -2 rec_auto/*.csv
grep -q "parecord" rec_auto/*.csv || fail "CSV does not mention the app using the mic"

step "record with audio + gain change mid-recording"
rm -rf rec
( sleep 3; pactl set-source-volume ci_sine_mic 50% ) &
$BIN record ci_sine_mic --audio --seconds 7 --out rec | tr '\r' '\n' | grep -v "^⏺" | tail -20
pactl set-source-volume ci_sine_mic 100%
CSV=$(ls rec/*.csv)
WAV=$(ls rec/*.wav)
python3 - "$CSV" "$WAV" <<'EOF2'
import csv, sys, wave
rows = list(csv.DictReader(open(sys.argv[1])))
for r in rows:
    print(r["t"], r["rms_db"], r["peak_db"], r["gain_pct"], r["status"])
assert len(rows) >= 6, len(rows)
early = [float(r["rms_db"]) for r in rows[1:3]]
print("early rms:", early, "gains:", [r["gain_pct"] for r in rows])
# 0.5-amplitude sine: RMS = 0.354 -> -9.0 dBFS
assert all(-10.5 < x < -7.5 for x in early), early
assert rows[1]["gain_pct"] == "100.0" and rows[-1]["gain_pct"] == "50.0"
late = float(rows[-2]["rms_db"])
# 50% on PulseAudio's cubic scale = -18 dB
assert late < early[0] - 12, (late, early)
w = wave.open(sys.argv[2])
dur = w.getnframes() / w.getframerate()
print("wav:", w.getframerate(), "Hz", w.getnchannels(), "ch", round(dur, 2), "s")
assert w.getnchannels() == 1 and w.getframerate() in (16000, 44100)
assert 5.0 < dur < 8.5, dur
print("recording OK")
EOF2
$BIN analyze "$CSV" | tee report.txt
grep -q "OS gain lowered from 100% to 50%" report.txt || fail "gain change not found in report"
$BIN analyze "$CSV" --json | python3 -c 'import json,sys; r=json.load(sys.stdin); assert any(f["kind"]=="gain_change" for f in r["findings"])'

# Peak of a raw s16le mono capture file.
peak_of() { python3 -c '
import sys, struct
d = open(sys.argv[1], "rb").read()
n = len(d) // 2
s = struct.unpack("<%dh" % n, d[: n * 2]) if n else []
print(max((abs(x) for x in s), default=0) / 32768)
' "$1"; }

# Output checks capture the sink monitor with parec (independent of our own meter).
step "speaker test tone"
parec --device=ci_headphones.monitor --format=s16le --rate=48000 --channels=1 --raw > tone_cap.raw &
PR=$!
sleep 0.5
$BIN test-tone ci_headphones --channel left
sleep 0.3
kill $PR
P=$(peak_of tone_cap.raw)
echo "captured peak: $P"
# 0.25 amplitude on the left only, downmixed to mono = 0.125
python3 -c "import sys; p=float('$P'); sys.exit(0 if 0.08 < p < 0.3 else 1)" || fail "test tone peak $P"

step "mic test (record the sine mic, play it back on ci_speakers)"
parec --device=ci_speakers.monitor --format=s16le --rate=48000 --channels=1 --raw > mic_cap.raw &
PR=$!
sleep 0.5
$BIN mic-test --input ci_sine_mic --output ci_speakers --seconds 2
sleep 0.3
kill $PR
P=$(peak_of mic_cap.raw)
echo "captured peak: $P"
# The 0.5-amplitude sine, recorded and played back
python3 -c "import sys; p=float('$P'); sys.exit(0 if 0.3 < p < 0.7 else 1)" || fail "mic test playback peak $P"

step "per-app mixer"
pacat --playback --device=ci_headphones --format=s16le --rate=48000 --channels=2 tone.raw &
PA=$!
sleep 1
$BIN apps | tee apps.txt
grep -qi "pacat\|paplay\|tone" apps.txt || { pactl list sink-inputs short; fail "player not listed"; }
APP=$(head -1 apps.txt | awk '{print $1}')
$BIN app-volume "$APP" 50
pactl list sink-inputs | grep -q "50%" || { pactl list sink-inputs; fail "app volume not applied"; }
$BIN app-mute "$APP" on
pactl list sink-inputs | grep -q "Mute: yes" || fail "app not muted"
kill $PA 2>/dev/null || true

step "errors"
if $BIN set-default output does-not-exist 2>err.txt; then fail "expected failure"; fi
grep -q "no output device matches" err.txt || { cat err.txt; fail "bad error message"; }

echo; echo "ALL LINUX INTEGRATION TESTS PASSED"
