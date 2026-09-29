# OpenRazer

An up to date GTK4 app for fully controlling Razer Blade laptops on Linux — fans, power profiles, CPU/GPU boost,
battery health, and keyboard lighting with the full suite of razer effects and individual key color configuration capability — with no kernel modules and no DKMS. Built for
**[Omarchy](https://omarchy.org)** (Hyprland) and tested on a **Razer Blade 15 Advanced (Early
2022)**; however it also runs on Ubuntu, Fedora, Mint, and Arch.

# Omarchy
It installs as the `openrazer` package and shows up in your app launcher as **Razer Settings**.

## Features

| Feature | Description |
|---|---|
| Keyboard lighting | Off, Static, Wave, Breathing, Reactive, Spectrum, Starlight, Wheel, Audio Meter, Stars, Ripple, CPU Temperature, plus brightness |
| Per-key painting | Click-to-paint grid sized to your model (6 × 16 on most Blades), with key labels for your keyboard layout, plus Fill All, Random All, and Clear All |
| Fan control | Automatic, or a fixed RPM within your model's range |
| Power profiles | Balanced, Gaming, Creator, Silent, or Custom with separate CPU/GPU boost |
| Battery health | Cap charging at 50–80% to extend battery lifespan |
| AC / battery profiles | Separate settings for plugged-in and on-battery, switched automatically |
| System monitor | Live CPU/iGPU/dGPU temperatures, power draw, utilization, and battery |
| Omarchy theming | Picks up your active Omarchy theme's colors and font |
| Desktop widgets | An Omarchy bar widget and a KDE Plasma widget for quick access |
| Device details | Keyboard firmware version, keyboard layout, lighting grid size, and BIOS version on the About page |

**Lighting notes**

- **Static and Breathing** render through the per-key engine, because some firmware accepts the
  native commands but never changes the visible color.
- **Wheel** is a software-rendered rotating rainbow, since some firmware lacks Razer's native one.
- **Audio Meter** is a real-time spectrum (bass → treble across the columns) with adjustable
  sensitivity, decay, and brightness, colored as Rainbow, Static, or Intensity Gradient.
- **Stars** re-rolls every key to a random color on an interval. Speed 1–4 maps to
  1.00 / 0.75 / 0.50 / 0.25 seconds.
- **Ripple** sends a ring of light outward from each key you press, in one static color, a
  rainbow that moves through the color wheel as the ring spreads, or a random color for every
  press. Speed 1–4 sets how fast the rings travel. While Ripple is active, the daemon reads the keyboard's key-press reports to find
  where each ring starts. It keeps only the key's position on the lighting grid: key codes are
  never logged, saved, or sent anywhere, and the keyboard stops being read as soon as you switch
  to another effect.
- **CPU Temperature** colors the whole keyboard by CPU temperature: blue at or below the
  "Cool At" temperature, through green and yellow, to red at or above "Hot At". The color
  eases between readings so brief spikes don't flash the keyboard.
- **Brightness** follows the keyboard: if you change it with the Fn keys, the app shows the new
  level and keeps it.
- Per-key effects (Wheel, Audio Meter, Stars, Ripple, CPU Temperature, and painting) need a model
  with per-key lighting; see [Supported Devices](#supported-devices).
- Open an issue if you'd like to see another lighting effect not listed.

**Known issues**

- **Logo LED:** on the Blade 15 Advanced (Early 2022), the firmware rejects or ignores every
  documented logo command, so the lid logo can't be controlled. Other models may work. If you get it to work open an issue with how so it can be implemented.

## Supported Devices

Supports 49 Razer laptops (but only tested on the 15 2022 model), from the 2015 Blade Stealth to
the 2026 Blade 18. 37 of them have per-key lighting, using the grid size OpenRazer lists for each
model: 6 × 16 on most, 6 × 17 on the Blade 16 (2025), 6 × 19 on the Blade 18 (2025), 6 × 22 on the
Blade Stealth (2015) and Blade Pro (Late 2016), and 6 × 25 on the Blade Pro (2017) models.

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
Paint-grid key labels were measured on the tested model; other 16-column models most likely match,
and wider grids are shown unlabeled.

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

## Command Line

`razer-cli` does everything the app does, from a terminal or a script. It talks to the same
background daemon, so the daemon must be running. Settings that are kept separately for AC and
battery take `ac` or `bat` as their first argument. Colors are three numbers from 0 to 255 (red,
green, blue). Add `--help` to any command to see its arguments and allowed values.

**Reading settings**

```bash
razer-cli read device            # model, lighting grid, keyboard firmware, keyboard layout
razer-cli read effect            # the lighting effect that's showing now
razer-cli read frame             # every key's color, one grid row per line
razer-cli read power ac          # power mode (and CPU/GPU boost in Custom)
razer-cli read fan ac            # fan setting (0 = automatic)
razer-cli read fan-rpm           # actual fan speed
razer-cli read brightness ac     # keyboard brightness, 0-100
razer-cli read logo ac           # logo mode
razer-cli read bho               # battery charge limit
razer-cli read sync              # whether AC and battery settings are linked
razer-cli read idle ac           # lights-off idle timeout
razer-cli read gpu               # GPUs, dGPU power management, envycontrol mode
```

**Performance and battery**

```bash
razer-cli write power ac 1               # 0 Balanced, 1 Gaming, 2 Creator, 3 Silent
razer-cli write fan ac 4000              # fixed RPM within your model's range; 0 = automatic
razer-cli write brightness bat 50        # keyboard brightness, 0-100
razer-cli write logo ac 1                # 0 off, 1 on, 2 breathing
razer-cli write bho on 80                # cap charging at 50-80% (multiples of 5)
razer-cli write bho off
razer-cli write sync on                  # link AC and battery settings
razer-cli write idle ac 10               # lights off after 10 idle minutes, 0 = never (GNOME only)
razer-cli write runtime-pm on            # let the dGPU power down when idle
razer-cli write gpu-mode hybrid          # hybrid, integrated, or nvidia (needs envycontrol)
```

**Lighting effects**

```bash
razer-cli standard-effect static 255 0 0
razer-cli standard-effect wave 1                         # direction 1 or 2
razer-cli standard-effect breathing 1 255 0 0            # 1 single color
razer-cli standard-effect breathing 2 255 0 0 0 0 255    # 2 two colors
razer-cli standard-effect breathing 3                    # 3 random colors
razer-cli standard-effect reactive 2 0 255 0             # speed 1-4, then color
razer-cli standard-effect starlight 1 2 255 255 255      # kind 1-3 as breathing, speed 1-3
razer-cli standard-effect spectrum
razer-cli standard-effect off

razer-cli wheel 1 50                          # direction 1 or 2, speed 0-100%
razer-cli stars 2                             # speed 1-4
razer-cli ripple 1 0 0 0 3                    # mode 1 rainbow, 2 static, 3 random; color; speed 1-4
razer-cli ripple 2 0 255 255 4
razer-cli temperature 45 90                   # blue at or below 45 °C, red at or above 90 °C
razer-cli audio-meter 1 0 255 0 100 30 100    # mode 1 rainbow, 2 static, 3 intensity; color;
                                              # sensitivity 1-200, decay 0-100, brightness 0-100
```

**Per-key painting**

Rows and columns count from 0 at the top-left key position; `read device` shows your grid size.

```bash
razer-cli paint fill 0 0 255          # every key one color
razer-cli paint key 3 5 255 0 0       # row 3, column 5
razer-cli paint random                # a random color on every key
razer-cli paint clear                 # every key off
```

Commands exit with status 0 on success and 1 on failure, so they can be used in scripts.

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
       "features": ["logo", "boost", "bho", "per_key_rgb"],
       "fan": [2200, 5000],
       "matrix": [6, 16]
   }
   ```
   Add `per_key_rgb` and `matrix` (rows, columns) only if your keyboard has per-key lighting;
   OpenRazer's `MATRIX_DIMS` for your model gives the grid size.
3. Add the PID to the `ATTRS{idProduct}` list in `data/udev/70-openrazer-hidraw.rules`.
4. Run `cargo test`, which checks the device list for duplicate PIDs, missing udev entries, and
   invalid grid sizes.
5. Reinstall: `./packaging/install.sh install`.

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
- [openrazer/openrazer](https://github.com/openrazer/openrazer) — the HID protocol reference, per-model lighting grid sizes and
  capabilities, keyboard layout ids, and the Ripple and CPU temperature effect designs
