#!/usr/bin/env bash
# Installer for the prebuilt tarball distribution (openrazer-<version>-x86_64.tar.gz).
# Expects to be run from the extracted tarball's own directory, which contains:
#   bin/, share/applications/, share/razercontrol/, systemd/, udev/
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if [ "$EUID" -ne 0 ]; then
    echo "Please run as root (sudo ./install.sh)"
    exit 1
fi

echo "Installing binaries..."
install -Dm755 bin/razer-daemon /usr/bin/razer-daemon
install -Dm755 bin/razer-cli /usr/bin/razer-cli
if [ -f bin/razer-settings ]; then
    install -Dm755 bin/razer-settings /usr/bin/razer-settings
fi

echo "Installing data files..."
install -Dm644 share/razercontrol/laptops.json /usr/share/razercontrol/laptops.json
# The rules file used to be 99-*; a stale copy would sort after this one and could re-grant world access.
rm -f /etc/udev/rules.d/99-hidraw-permissions.rules
install -Dm644 udev/70-openrazer-hidraw.rules /etc/udev/rules.d/70-openrazer-hidraw.rules
install -Dm644 systemd/razercontrol.service /usr/lib/systemd/user/razercontrol.service
if [ -f share/applications/io.github.stizzy98.openrazer.desktop ]; then
    install -Dm644 share/applications/io.github.stizzy98.openrazer.desktop \
        /usr/share/applications/io.github.stizzy98.openrazer.desktop
fi

echo "Reloading udev rules..."
udevadm control --reload-rules
udevadm trigger

echo "Installation complete. Log out and back in (or reboot) for udev rules to take effect,"
echo "then run: systemctl --user enable --now razercontrol"
