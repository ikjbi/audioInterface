#!/usr/bin/env python3
"""Exercise the real OS backend through the CLI against whatever devices exist.

For each direction it:
  * sets volume to 37% on the first adjustable device, reads it back, restores it
  * toggles mute twice (if the device has a mute control)
  * makes every device the default in turn, checks it took, then restores the original

Everything it changes is put back. Exits non-zero on any mismatch. With no
devices at all it passes with a warning (headless CI runners often have none),
unless --require-devices is given.

Usage: scripts/device-smoke.py path/to/audio-interface [--require-devices]
"""

import json
import subprocess
import sys

BIN = sys.argv[1] if len(sys.argv) > 1 else "target/debug/audio-interface"
REQUIRE = "--require-devices" in sys.argv
failures = []


def cli(*args):
    r = subprocess.run([BIN, *args], capture_output=True, text=True, timeout=60)
    if r.returncode != 0:
        raise RuntimeError(f"`{' '.join(args)}` failed: {r.stderr.strip()}")
    return r.stdout


def devices():
    return json.loads(cli("list", "--json"))


def find(direction, dev_id):
    return next((d for d in devices() if d["direction"] == direction and d["id"] == dev_id), None)


def check(cond, msg):
    print(("  ok   " if cond else "  FAIL ") + msg)
    if not cond:
        failures.append(msg)


all_devices = devices()
print(cli("list"))
if not all_devices:
    print("WARNING: no audio devices on this machine; nothing to test")
    sys.exit(1 if REQUIRE else 0)

for direction in ("input", "output"):
    devs = [d for d in all_devices if d["direction"] == direction]
    print(f"== {direction}: {len(devs)} device(s)")
    if not devs:
        continue

    adjustable = [d for d in devs if d["volume"] is not None]
    if adjustable:
        d = adjustable[0]
        original = d["volume"]
        try:
            cli("volume", direction, d["id"], "37")
            now = find(direction, d["id"])["volume"]
            check(abs(now - 0.37) < 0.02, f"volume of {d['name']!r} set to 37% (read back {now:.3f})")
        finally:
            cli("volume", direction, d["id"], f"{original * 100:.2f}")
        restored = find(direction, d["id"])["volume"]
        check(abs(restored - original) < 0.02, f"volume restored to {original:.3f} (read back {restored:.3f})")

    mutable = [d for d in devs if d["muted"] is not None]
    if mutable:
        d = mutable[0]
        original = d["muted"]
        try:
            cli("mute", direction, d["id"], "toggle")
            check(find(direction, d["id"])["muted"] == (not original), f"mute toggled on {d['name']!r}")
        finally:
            cli("mute", direction, d["id"], "on" if original else "off")
        check(find(direction, d["id"])["muted"] == original, "mute restored")

    original_default = next((d["id"] for d in devs if d["is_default"]), None)
    try:
        for d in devs:
            cli("set-default", direction, d["id"])
            now = [x["id"] for x in devices() if x["direction"] == direction and x["is_default"]]
            check(now == [d["id"]], f"default {direction} -> {d['name']!r} (defaults now {now})")
    finally:
        if original_default:
            cli("set-default", direction, original_default)
    if original_default:
        now = [x["id"] for x in devices() if x["direction"] == direction and x["is_default"]]
        check(now == [original_default], "original default restored")

if failures:
    print(f"\n{len(failures)} FAILURE(S)")
    sys.exit(1)
print("\nDEVICE SMOKE TEST PASSED")
