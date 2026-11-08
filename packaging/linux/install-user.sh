#!/usr/bin/env sh
# Install Audio Interface for the current user only. No root/sudo needed.
#   ./install-user.sh            install (or update)
#   ./install-user.sh --autostart  also start it at login
#   ./install-user.sh --uninstall  remove it again
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
CONF="${XDG_CONFIG_HOME:-$HOME/.config}"
APPS="$DATA/applications"
ICONS="$DATA/icons/hicolor/512x512/apps"
DESKTOP_ID=com.ikjbi.AudioInterface.desktop

if [ "${1:-}" = "--uninstall" ]; then
  rm -f "$BIN_DIR/audio-interface" "$APPS/$DESKTOP_ID" "$ICONS/audio-interface.png" "$CONF/autostart/$DESKTOP_ID"
  echo "Removed. (Settings in $CONF/audiointerface and your recordings were kept.)"
  exit 0
fi

mkdir -p "$BIN_DIR" "$APPS" "$ICONS"
install -m 755 "$HERE/audio-interface" "$BIN_DIR/audio-interface"
install -m 644 "$HERE/icon.png" "$ICONS/audio-interface.png"
sed "s|^Exec=.*|Exec=$BIN_DIR/audio-interface|" "$HERE/audio-interface.desktop" > "$APPS/$DESKTOP_ID"
chmod 644 "$APPS/$DESKTOP_ID"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPS" >/dev/null 2>&1 || true

if [ "${1:-}" = "--autostart" ]; then
  mkdir -p "$CONF/autostart"
  cp "$APPS/$DESKTOP_ID" "$CONF/autostart/$DESKTOP_ID"
  echo "Autostart enabled."
fi

echo "Installed to $BIN_DIR/audio-interface"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo "Note: $BIN_DIR is not on your PATH. Add this to ~/.profile:  export PATH=\"$BIN_DIR:\$PATH\"" ;;
esac
echo "Find \"Audio Interface\" in your application menu, or run: audio-interface"
