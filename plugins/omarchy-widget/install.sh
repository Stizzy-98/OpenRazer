#!/usr/bin/env bash
# Installs the Razer Lighting Quickshell plugin into Omarchy's bar.
set -euo pipefail

PLUGIN_ID="razercontrol.lighting"
SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST_DIR="$HOME/.config/omarchy/plugins/$PLUGIN_ID"

if ! command -v razer-cli >/dev/null 2>&1; then
    echo "Warning: razer-cli not found on PATH. Install the main package first" >&2
    echo "(see the project README), otherwise the widget's buttons will fail." >&2
fi

mkdir -p "$(dirname "$DEST_DIR")"
rm -rf "$DEST_DIR"
cp -r "$SRC_DIR" "$DEST_DIR"
rm -f "$DEST_DIR/install.sh"

echo "Installed to $DEST_DIR"

if command -v omarchy-shell >/dev/null 2>&1; then
    omarchy-shell shell rescanPlugins || true
fi

if command -v omarchy >/dev/null 2>&1; then
    echo "Enabling the plugin..."
    omarchy plugin enable "$PLUGIN_ID" || echo "Run 'omarchy plugin enable $PLUGIN_ID' manually to finish."
else
    cat <<EOF
Now run:
  omarchy-shell shell rescanPlugins
  omarchy plugin enable $PLUGIN_ID
EOF
fi
