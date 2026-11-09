#!/usr/bin/env bash
# Diagnostics for the test-tone path on PulseAudio (run by linux-debug.yml).
set -x
BIN=${1:-target/debug/audio-interface}
peak_of() { python3 -c '
import sys, struct
d = open(sys.argv[1], "rb").read()
n = len(d) // 2
s = struct.unpack("<%dh" % n, d[: n * 2]) if n else []
print("samples", n, "peak", max((abs(x) for x in s), default=0) / 32768)
' "$1"; }

# 1. Independent capture of the sink monitor while the tone plays.
parec --device=ci_headphones.monitor --format=s16le --rate=48000 --channels=1 --raw > cap.raw &
PR=$!
sleep 0.5
RUST_LOG=debug $BIN test-tone ci_headphones --channel both &
TT=$!
sleep 0.8
pactl list sink-inputs
wait $TT
echo "test-tone exit=$?"
sleep 0.5
kill $PR
peak_of cap.raw

# 2. Same with a known-good player, for comparison.
python3 -c '
import math, struct, sys
r = 48000
sys.stdout.buffer.write(b"".join(struct.pack("<h", int(8000 * math.sin(2 * math.pi * 440 * i / r))) for i in range(r)))
' > ref.raw
parec --device=ci_headphones.monitor --format=s16le --rate=48000 --channels=1 --raw > cap2.raw &
PR=$!
sleep 0.5
pacat --playback --device=ci_headphones --format=s16le --rate=48000 --channels=1 ref.raw
sleep 0.3
kill $PR
peak_of cap2.raw
