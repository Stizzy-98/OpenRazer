Name:           openrazer
Version:        0.4.0
Release:        1%{?dist}
Summary:        Razer Laptop Control

License:        GPLv2

# Rust binaries are already stripped; skip debuginfo generation
%global debug_package %{nil}
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  rust
BuildRequires:  cargo
BuildRequires:  dbus-devel
BuildRequires:  libusb1-devel
BuildRequires:  hidapi-devel
BuildRequires:  gtk4-devel
BuildRequires:  libadwaita-devel
BuildRequires:  systemd-devel
BuildRequires:  glib2-devel
BuildRequires:  graphene-devel
BuildRequires:  pango-devel
BuildRequires:  cairo-devel
BuildRequires:  gdk-pixbuf2-devel

Requires:       dbus
Requires:       hidapi
Requires:       gtk4
Requires:       libadwaita

%description
A Linux userspace application to control Razer Blade laptops. No kernel modules (DKMS) required!
Features a modern GTK4/libadwaita interface for fan control, power modes, keyboard lighting, and battery health optimization.

%prep
%autosetup

%build
cargo build --release

%install
rm -rf $RPM_BUILD_ROOT
install -D -m 755 target/release/razer-settings $RPM_BUILD_ROOT%{_bindir}/razer-settings
install -D -m 755 target/release/razer-cli $RPM_BUILD_ROOT%{_bindir}/razer-cli
install -D -m 755 target/release/daemon $RPM_BUILD_ROOT%{_bindir}/razer-daemon
install -D -m 644 data/gui/io.github.stizzy98.openrazer.desktop $RPM_BUILD_ROOT%{_datadir}/applications/io.github.stizzy98.openrazer.desktop
install -D -m 644 data/gui/icon.png $RPM_BUILD_ROOT%{_datadir}/pixmaps/io.github.stizzy98.openrazer.png
install -D -m 644 data/gui/icon.png $RPM_BUILD_ROOT%{_datadir}/icons/hicolor/512x512/apps/io.github.stizzy98.openrazer.png
install -D -m 644 data/devices/laptops.json $RPM_BUILD_ROOT%{_datadir}/razercontrol/laptops.json
install -D -m 644 data/udev/70-openrazer-hidraw.rules $RPM_BUILD_ROOT%{_udevrulesdir}/70-openrazer-hidraw.rules
install -D -m 644 data/services/systemd/razercontrol.service $RPM_BUILD_ROOT%{_userunitdir}/razercontrol.service

%files
%{_bindir}/razer-settings
%{_bindir}/razer-cli
%{_bindir}/razer-daemon
%{_datadir}/applications/io.github.stizzy98.openrazer.desktop
%{_datadir}/pixmaps/io.github.stizzy98.openrazer.png
%{_datadir}/icons/hicolor/512x512/apps/io.github.stizzy98.openrazer.png
%{_datadir}/razercontrol/laptops.json
%{_udevrulesdir}/70-openrazer-hidraw.rules
%{_userunitdir}/razercontrol.service
%license LICENSE
%doc README.md

%post
udevadm control --reload-rules || :
udevadm trigger --subsystem-match=hidraw --action=change || :
%systemd_user_post razercontrol.service
# Enable for all users so daemon starts on login
systemctl --global enable razercontrol.service 2>/dev/null || :
# Start immediately for the installing user
if [ -n "$SUDO_USER" ]; then
    _UID=$(id -u "$SUDO_USER" 2>/dev/null)
    if [ -n "$_UID" ] && [ -d "/run/user/$_UID" ]; then
        su -s /bin/sh "$SUDO_USER" -c \
            "XDG_RUNTIME_DIR=/run/user/$_UID systemctl --user daemon-reload; \
             XDG_RUNTIME_DIR=/run/user/$_UID systemctl --user start razercontrol.service" \
            2>/dev/null || :
    fi
fi

%preun
%systemd_user_preun razercontrol.service
if [ $1 -eq 0 ]; then
    systemctl --global disable razercontrol.service 2>/dev/null || :
fi

%postun
%systemd_user_postun_with_restart razercontrol.service

%changelog
* Sat Sep 26 2026 Stizzy-98 <stizzy98@outlook.com> - 0.4.0-1
- Wire the full native Chroma effect suite into the GUI (Off/Static/Wave/Breathing/
  Reactive/Spectrum/Starlight) with corrected direction/speed/color-mode encodings
- Real per-key RGB painting (click-to-paint grid, Fill/Random/Clear All); fix custom-frame
  HID payload offset/size bug and a board-size bug (this device is 6 rows x 16 columns, not 15)
- Add software Wheel effect (circular rainbow) and a real-time FFT-based Audio Meter effect
  with configurable sensitivity/decay/brightness and 3 color modes
- GUI reads the active Omarchy theme's colors/font and matches its look automatically
- Add software Stars effect (per-key random colors re-rolled on a 1-4 speed interval)
- Reorder tabs (Lighting first)
