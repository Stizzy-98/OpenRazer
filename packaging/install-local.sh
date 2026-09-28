#!/bin/bash
# Local installation script for Razer Control

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# This script now lives in packaging/, one level below the repo root, where Cargo.toml/src/data
# live directly (no more razer_control_gui/ nesting).
BUILD_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

if [ "$EUID" -eq 0 ]; then
    echo "Please do not run as root (sudo will be used where needed)"
    exit 1
fi

echo "Installing Razer Control..."

RAZER_SETTINGS_BIN="$BUILD_DIR/target/release/razer-settings"
RAZER_DAEMON_BIN="$BUILD_DIR/target/release/daemon"
RAZER_CLI_BIN="$BUILD_DIR/target/release/razer-cli"

if [ -f "$RAZER_SETTINGS_BIN" ] && [ -f "$RAZER_DAEMON_BIN" ] && [ -f "$RAZER_CLI_BIN" ]; then
    echo "Release binaries found, skipping build."
else
    echo "Release binaries missing, building with cargo..."
    cargo build --release --manifest-path "$BUILD_DIR/Cargo.toml"
fi

# Install binaries (consistent with deb/rpm package names)
echo "Installing binaries to /usr/bin..."
sudo install -Dm755 "$RAZER_SETTINGS_BIN" /usr/bin/razer-settings
sudo install -Dm755 "$RAZER_DAEMON_BIN" /usr/bin/razer-daemon
sudo install -Dm755 "$RAZER_CLI_BIN" /usr/bin/razer-cli

# Install desktop file
echo "Installing desktop entry..."
sudo install -Dm644 "$BUILD_DIR/data/gui/io.github.stizzy98.openrazer.desktop" /usr/share/applications/io.github.stizzy98.openrazer.desktop

echo "Installing SVG icon..."
sudo mkdir -p /usr/share/icons/hicolor/scalable/apps/
sudo install -Dm644 "$BUILD_DIR/data/gui/io.github.stizzy98.openrazer.svg" /usr/share/icons/hicolor/scalable/apps/io.github.stizzy98.openrazer.svg

# Install udev rules
echo "Installing udev rules..."
# The rules file used to be 99-*; a stale copy would sort after this one and could re-grant world access.
sudo rm -f /etc/udev/rules.d/99-hidraw-permissions.rules
sudo install -Dm644 "$BUILD_DIR/data/udev/70-openrazer-hidraw.rules" /etc/udev/rules.d/70-openrazer-hidraw.rules

# Install systemd user service
echo "Installing systemd user service..."
sudo install -Dm644 "$BUILD_DIR/data/services/systemd/razercontrol.service" /usr/lib/systemd/user/razercontrol.service

# Install device configuration
echo "Installing device configuration..."
sudo mkdir -p /usr/share/razercontrol
sudo install -Dm644 "$BUILD_DIR/data/devices/laptops.json" /usr/share/razercontrol/laptops.json

# Create config directory
mkdir -p ~/.local/share/razercontrol

# Reload udev and systemd
echo "Reloading udev rules..."
sudo udevadm control --reload-rules
sudo bash <<'EOF'
# Reset Razer hidraw nodes that an older rule left world-accessible (or with a stale, masked ACL),
# then re-trigger so uaccess grants a fresh ACL to the logged-in user.
for u in /sys/class/hidraw/hidraw*/device/uevent; do
    grep -qi '^HID_ID=0003:00001532:' "$u" || continue
    dev="/dev/$(basename "$(dirname "$(dirname "$u")")")"
    chmod go-rwx "$dev"
    command -v setfacl >/dev/null && setfacl -b "$dev"
done
udevadm trigger --subsystem-match=hidraw --action=change
udevadm settle
EOF

echo "Reloading systemd user daemon..."
systemctl --user daemon-reload

# Enable and start the user service
echo "Enabling and starting razercontrol daemon..."
systemctl --user enable razercontrol.service
systemctl --user restart razercontrol.service


# Validating icon cache
if command -v gtk-update-icon-cache &> /dev/null; then
    echo "Updating GTK icon cache..."
    sudo gtk-update-icon-cache -f -t /usr/share/icons/hicolor || true
fi

if command -v kbuildsycoca5 &> /dev/null; then
    echo "Updating KDE configuration cache..."
    kbuildsycoca5 --noincremental &> /dev/null || true
elif command -v kbuildsycoca6 &> /dev/null; then
    echo "Updating KDE configuration cache..."
    kbuildsycoca6 --noincremental &> /dev/null || true
fi

# Update Plasmoid if detected
PLASMOID_DIR="$HOME/.local/share/plasma/plasmoids/io.github.stizzy98.openrazer.plasmoid"
if [ -d "$PLASMOID_DIR" ]; then
    echo "Updating KDE Plasmoid..."
    cp -r "$BUILD_DIR/plugins/kde-widget/package/"* "$PLASMOID_DIR/" 2>/dev/null || true
fi

echo ""
echo "Installation complete!"
