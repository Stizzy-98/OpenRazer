# OpenRazer

An up to date GTK4 app for fully controlling Razer Blade laptops on Linux — fans, power profiles, CPU/GPU boost,
battery health, and keyboard lighting with full suite of razer effects and individual key color configuration capability — with no kernel modules and no DKMS. Built for
**[Omarchy](https://omarchy.org)** (Hyprland) and tested on a **Razer Blade 15 Advanced (Early
2022)**; however it also runs on Ubuntu, Fedora, Mint, and Arch.

# Omarchy
It installs as the `openrazer` package and shows up in your app launcher as **Razer Settings**.

## Features

| Feature | Description |
|---|---|
| Keyboard lighting | Off, Static, Wave, Breathing, Reactive, Spectrum, Starlight, Wheel, Audio Meter, Stars, Ripple, plus brightness |
| Per-key painting | Click-to-paint 96-key grid with Fill All, Random All, and Clear All |
| Fan control | Automatic, or a fixed RPM within your model's range |
| Power profiles | Balanced, Gaming, Creator, Silent, or Custom with separate CPU/GPU boost |
| Battery health | Cap charging at 50–80% to extend battery lifespan |
| AC / battery profiles | Separate settings for plugged-in and on-battery, switched automatically |
| System monitor | Live CPU/iGPU/dGPU temperatures, power draw, utilization, and battery |
| Omarchy theming | Picks up your active Omarchy theme's colors and font |
| Desktop widgets | An Omarchy bar widget and a KDE Plasma widget for quick access |

**Lighting notes**

- **Static and Breathing** render through the per-key engine, because some firmware accepts the
  native commands but never changes the visible color.
- **Wheel** is a software-rendered rotating rainbow, since some firmware lacks Razer's native one.
- **Audio Meter** is a real-time spectrum (bass → treble across the columns) with adjustable
  sensitivity, decay, and brightness, colored as Rainbow, Static, or Intensity Gradient.
- **Stars** re-rolls every key to a random color on an interval. Speed 1–4 maps to
  1.00 / 0.75 / 0.50 / 0.25 seconds.
- **Ripple** sends a ring of light outward from each key you press, in one static color or a
  rainbow that moves through the color wheel as the ring spreads. Speed 1–4 sets how fast the
  rings travel. While Ripple is active, the daemon reads the keyboard's key-press reports to find
  where each ring starts. It keeps only the key's position on the lighting grid: key codes are
  never logged, saved, or sent anywhere, and the keyboard stops being read as soon as you switch
  to another effect.

**Known issues**

- **Logo LED:** on the Blade 15 Advanced (Early 2022), the firmware rejects or ignores every
  documented logo command, so the lid logo can't be controlled. Other models may work. If you get it to work open an issue with how so it can be implemented.

## Supported Devices

Supports 50 Razer laptops (but only tested on the 15 2022 model), from the 2015 Blade Stealth to the 2025 Blade 16.

<details>
<summary>Device list</summary>

| Model | Year | USB PID | Status |
|-------|------|---------|--------|
| Blade Stealth | 2015–2020 | Various | Supported |
| Blade 15 | 2016–2023 | Various | Supported |
| Blade Pro | 2017–2021 | Various | Supported |
| Blade 14 | 2021–2025 | Various | Supported |
| Blade 16 | 2023–2025 | Various | Supported |
| Blade 17 | 2022 | 028B | Supported |
| Blade 18 | 2023–2025 | Various | Supported |
| Razer Book 13 | 2020 | 026A | Supported |
| Blade 15 Advanced (Early 2022) | 2022 | 028A | Tested |

</details>

"Supported" means the model is in the device database. The lighting features were verified on the
tested model, so other models may hit firmware quirks. Open an issue if you do so we can work to fix it.

For troubleshooting find your model's USB PID:
```bash
lsusb | grep -i razer
# Bus XXX Device XXX: ID 1532:XXXX Razer USA, Ltd  ← XXXX is your USB PID
```

## Install

Each release on the GitHub Releases page includes the packages and a `SHA256SUMS` file. Verify your
download before installing:

```bash
sha256sum -c SHA256SUMS --ignore-missing
```

### Arch Linux / Omarchy

Build from source:

```bash
sudo pacman -S rust cargo dbus libusb hidapi pkgconf systemd gtk4 libadwaita git
git clone <repository-url> openrazer
cd openrazer
./packaging/install.sh install
```

### Ubuntu / Debian / Linux Mint / Zorin OS

```bash
sudo apt install ./openrazer_0.4.0_amd64.deb
```

The background service starts on your next login.

### Fedora / RHEL / CentOS

```bash
sudo dnf install ./openrazer-0.4.0-1.fc41.x86_64.rpm
```

### NixOS

```nix
# flake inputs
inputs.openrazer = {
  url = "github:<owner>/<repo>";
  inputs.nixpkgs.follows = "nixpkgs";
};

# configuration
imports = [ inputs.openrazer.nixosModules.default ];
services.openrazer.enable = true;
```

### Any other distribution

Use the generic tarball, then start the service:

```bash
tar -xzf openrazer-0.4.0-x86_64.tar.gz
cd openrazer-0.4.0-x86_64
sudo ./install.sh
systemctl --user enable --now razercontrol
```

If your distro is too old for the tarball (see below), [build from source](#building-from-source)
instead.

If you'd like to see your prefered distro supported then open an issue please.

### Requirements

Prebuilt packages need glibc 2.39, GTK 4.10, and libadwaita 1.4 or newer: Ubuntu 24.04+, Zorin 18+,
Linux Mint 22+, Debian 13+, or Fedora 41+. Ubuntu 22.04, Zorin 17, Linux Mint 21, and older are not
supported.

## Usage

Launch **Razer Settings** from your app launcher, or run `razer-settings`. Launching it again while
it's open brings the existing window to the front.

Lighting is global. Power, fan, and keyboard brightness are kept separately for AC and battery —
use the AC/Battery toggle to edit each.

The app talks to a background daemon that runs as a systemd user service:

```bash
systemctl --user status razercontrol     # check it
systemctl --user restart razercontrol    # restart it
journalctl --user -u razercontrol -f     # follow its logs
```

## Desktop Widgets

**Omarchy bar widget** — a lighting preset switcher in the Omarchy top bar:

```bash
cd plugins/omarchy-widget
./install.sh
```

**KDE Plasma widget** — live temperatures, power draw, and battery status, plus one-click toggles
for power profile, fan, keyboard brightness, logo, and charge limit:

```bash
cd plugins/kde-widget
./install-plasmoid.sh
```

Then right-click your panel → Add Widgets → search "Razer Control". See
[plugins/kde-widget/README.md](plugins/kde-widget/README.md) for details.

Install the app before either widget.

## Troubleshooting

<details>
<summary>"No supported device found"</summary>

Your laptop's USB PID isn't in the device list — see
[Adding Support for New Devices](#adding-support-for-new-devices).
</details>

<details>
<summary>"Permission denied" opening the keyboard</summary>

Access to the keyboard is granted to the logged-in desktop user. Check that your user has it:

```bash
getfacl /dev/hidraw* 2>/dev/null | grep "user:$USER"
```

If nothing is printed, re-apply the udev rules, then log out and back in:

```bash
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=hidraw --action=change
```

Access only applies to local desktop sessions, not SSH.
</details>

<details>
<summary>The app can't connect to the daemon</summary>

```bash
systemctl --user start razercontrol
journalctl --user -u razercontrol -n 50
```

If the log says another daemon is responding, a second copy is already running — stop it before
starting the service. A lot has been put in to preventing this from happening open an issue if it occurs consistently.
</details>

## Uninstall

From the repository:
```bash
./packaging/install.sh uninstall
```

Package installs: remove the `openrazer` package with your package manager.

## Development

### Building from source

Install the Rust toolchain (≥ 1.88, via [rustup](https://rustup.rs/) if your distro's is older)
and the development packages:

```bash
# Fedora / RHEL / CentOS
sudo dnf install -y rust cargo dbus-devel libusb1-devel hidapi-devel \
    pkgconf systemd-devel gtk4-devel libadwaita-devel git

# Ubuntu / Debian
sudo apt install -y rustc cargo libdbus-1-dev libusb-1.0-0-dev libhidapi-dev \
    pkg-config libsystemd-dev libgtk-4-dev libadwaita-1-dev git

# Arch Linux / Omarchy
sudo pacman -S rust cargo dbus libusb hidapi pkgconf systemd gtk4 libadwaita git
```

Then run `./packaging/install.sh install` from a clone of this repository. GTK 4 ≥ 4.12 and
libadwaita ≥ 1.5 are needed to build the GUI.

### Adding Support for New Devices

1. Find your USB PID: `lsusb | grep -i razer` (e.g. `ID 1532:02c6` → PID `02c6`).
2. Add an entry to `data/devices/laptops.json`:
   ```json
   {
       "name": "Blade XX 20XX",
       "vid": "1532",
       "pid": "YOUR_PID_HERE",
       "features": ["logo", "boost", "bho"],
       "fan": [2200, 5000]
   }
   ```
3. Add the PID to the `ATTRS{idProduct}` list in `data/udev/70-openrazer-hidraw.rules`.
4. Reinstall: `./packaging/install.sh install`.

## Warning

Provided as-is, with no warranty. Not affiliated with Razer Inc. This is a community project with
no official support; use it at your own risk to your hardware.

## License

GPL-2.0 — see [LICENSE](LICENSE).

## Credits

This project was started off of portions of earlier unmaintained open-source work, whose authors I'd like to credit:

- [Razer-Linux/razer-laptop-control-no-dkms](https://github.com/Razer-Linux/razer-laptop-control-no-dkms) — the original daemon and HID protocol work
- [encomjp/razer-control-revived](https://github.com/encomjp/razer-control-revived) — the GTK4 app, HID fixes, and packaging this codebase was built from
- [@johva1312](https://github.com/johva1312) — HID device init fallbacks and partial socket-read fix
- [@sini](https://github.com/sini) — NixOS flake fixes
