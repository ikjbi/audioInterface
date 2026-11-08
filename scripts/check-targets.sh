#!/usr/bin/env bash
# Type-check and lint the crate for all three desktop OSes from one machine
# (handy on macOS). Needs: rustup targets x86_64-pc-windows-msvc and
# x86_64-unknown-linux-gnu, plus pkg-config.
#
# Linux: cpal links ALSA via pkg-config. `cargo check` never links, so a stub
# alsa.pc is enough to get through the build script when cross-checking.
set -euo pipefail
cd "$(dirname "$0")/.."

stub=$(mktemp -d)
trap 'rm -rf "$stub"' EXIT
printf 'Name: alsa\nDescription: stub\nVersion: 1.2.10\nLibs: -lasound\nCflags:\n' > "$stub/alsa.pc"

echo "== host ==";    cargo clippy --all-targets -- -D warnings
echo "== windows =="; cargo clippy --all-targets --target x86_64-pc-windows-msvc -- -D warnings
echo "== linux ==";   PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_PATH="$stub" PKG_CONFIG_SYSROOT_DIR=/ \
  cargo clippy --all-targets --target x86_64-unknown-linux-gnu -- -D warnings
echo "All targets OK"
