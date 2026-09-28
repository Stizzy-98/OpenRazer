#!/usr/bin/env bash

# All paths below (cargo build, target/, data/) are relative to the repository root, not this
# script's own location, since it lives in packaging/ - always run from there regardless of the
# caller's cwd.
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

install() {
    echo "Building the project..."
    if [[ "$2" != "--no-gui" ]]; then
        cargo build --release
    else
        cargo build --release --no-default-features
    fi

    if [ $? -ne 0 ]; then
        echo "An error occurred while building the project"
        exit 1
    fi

    echo "Stopping the service..."
    systemctl --user stop razercontrol

    echo "Installing daemon and service files..."
    mkdir -p ~/.local/share/razercontrol
    sudo bash <<EOF
        mkdir -p /usr/share/razercontrol
        cp target/release/razer-cli /usr/bin/
        cp target/release/daemon /usr/bin/razer-daemon
        cp data/devices/laptops.json /usr/share/razercontrol/
        # The rules file used to be 99-*; a stale copy would sort after this one and could
        # re-grant world access.
        rm -f /etc/udev/rules.d/99-hidraw-permissions.rules
        cp data/udev/70-openrazer-hidraw.rules /etc/udev/rules.d/
        cp data/services/systemd/razercontrol.service /usr/lib/systemd/user/
        udevadm control --reload-rules
EOF
    if [ $? -ne 0 ]; then
        echo "An error occurred while installing the files"
        exit 1
    fi

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

    if [ $? -ne 0 ]; then
        echo "An error occurred while installing the files"
        exit 1
    fi

    if [[ "$2" != "--no-gui" ]]; then
        echo "Installing gui files..."
        sudo bash <<EOF
            cp target/release/razer-settings /usr/bin/
            if ls /usr/share/applications/*.desktop 1> /dev/null 2>&1; then
                # We only install the desktop file if there are already desktop
                # files on the system
                cp data/gui/io.github.stizzy98.openrazer.desktop /usr/share/applications/
            fi
            install -Dm644 data/gui/io.github.stizzy98.openrazer.svg /usr/share/icons/hicolor/scalable/apps/io.github.stizzy98.openrazer.svg
EOF
        if [ $? -ne 0 ]; then
            echo "An error occurred while installing the files"
            exit 1
        fi
    fi

    echo "Starting the service..."
    systemctl --user daemon-reload
    systemctl --user enable --now razercontrol

    echo "Installation complete"
}

uninstall() {
    echo "Stopping the service..."
    systemctl --user disable --now razercontrol

    echo "Uninstalling the files..."
    sudo bash <<EOF
        rm -f /usr/bin/razer-cli
        rm -f /usr/bin/razer-settings
        rm -f /usr/bin/razer-daemon
        rm -f /usr/share/applications/io.github.stizzy98.openrazer.desktop
        rm -f /usr/share/icons/hicolor/scalable/apps/io.github.stizzy98.openrazer.svg
        rm -f /usr/share/razercontrol/laptops.json
        rm -f /usr/lib/systemd/user/razercontrol.service
        rm -f /etc/udev/rules.d/70-openrazer-hidraw.rules
        udevadm control --reload-rules
EOF

    if [ $? -ne 0 ]; then
        echo "An error occurred while uninstalling the files"
        exit 1
    fi

    echo "Uninstalled"
}

main() {
    if [ "$EUID" -eq 0 ]; then
        echo "Please do not run as root"
        exit 1
    fi

    if ! pidof systemd >/dev/null 2>&1; then
        echo "Unsupported init system: a systemd user session is required"
        exit 1
    fi

    case $1 in
    install)
        install "$@"
        ;;
    uninstall)
        uninstall
        ;;
    *)
        echo "Usage: $0 {install|uninstall} [--no-gui]"
        exit 1
        ;;
    esac
}

main "$@"
