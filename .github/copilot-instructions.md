# Project Guidelines

## Overview

Razer Control — a GUI-only Linux application for controlling Razer Blade laptops without kernel modules/DKMS. A user-level daemon owns the hardware; the GTK4 GUI and the desktop widgets talk to it over a Unix socket using bincode-serialized IPC.

## Architecture

```
┌──────────────────┐   ┌──────────────────────────────────┐
│  razer-settings  │   │ Omarchy bar widget / KDE widget  │
│  (GTK4 + libadw) │   │ (QML) ──runs──▶ razer-cli helper │
└────────┬─────────┘   └────────────────┬─────────────────┘
         │ bincode                      │ bincode
         └───────────────┬──────────────┘
                         ▼
     ┌─────────────────────────────────────────────┐
     │  razer-daemon  (systemd --user service)     │
     │  socket: $XDG_RUNTIME_DIR/razercontrol-socket│
     │  hidapi → Razer keyboard/EC (USB HID iface 0)│
     └─────────────────────────────────────────────┘
```

`razer-cli` is an **internal helper** that exists only so the QML widgets (which can't speak bincode) can reach the daemon. It is not a user-facing CLI — don't document it in the README.

**Key boundaries:**
- `src/daemon/` — daemon: HID control (`device.rs`), config persistence, D-Bus integration (UPower, Mutter, login1)
- `src/daemon/kbd/` — per-key RGB engine (`Effect` trait, `EffectManager` layer/mask system, `board.rs` 6×16 key model)
- `src/daemon/audio.rs` — audio capture (`parec`) + FFT feeding the Audio Meter effect
- `src/razer-settings/` — GTK4 + libadwaita GUI (no direct hardware access); `omarchy_theme.rs` generates CSS from the active Omarchy theme
- `src/cli/` — the widget helper binary (clap)
- `src/lib.rs` + `src/comms.rs` — the shared `service` library: `DaemonCommand`/`DaemonResponse` IPC, `SupportedDevice`, device-file path
- `plugins/omarchy-widget/` — Quickshell bar-widget plugin (QML)
- `plugins/kde-widget/` — KDE Plasma 6 plasmoid (QML only)

**Config storage:** `~/.local/share/razercontrol/daemon.json` (AC/Battery profiles) and `effects.json` (active software effect layers).

**Device database:** [data/devices/laptops.json](data/devices/laptops.json) — array of `{name, vid, pid, features[], fan[min,max], matrix?[rows,cols]}` (`matrix` only with `per_key_rgb`).

**App ID:** `io.github.stizzy98.openrazer` (GApplication ID, desktop file, icon).

## Build and Test

```bash
cargo build --release
cargo test --release
cargo clippy --all-targets -- -D warnings   # CI gate
cargo fmt --check                           # CI gate

./packaging/install-local.sh                # install binaries + udev + systemd user service
```

**Toolchain:** Rust stable ≥ 1.88 (edition 2024, let-chains).

Unit tests cover pure logic (packet CRC, power-mode mapping). Anything touching the hardware still needs verification on a real laptop.

**Packaging:** RPM ([packaging/fedora/razercontrol.spec](packaging/fedora/razercontrol.spec)), DEB, and tarball are built in GitHub Actions on tag push (`v*`). **Nix:** `flake.nix` provides a NixOS module with systemd + udev integration.

## Conventions

- **Protocol quirks are real:** several native Chroma commands are ACKed by this hardware's firmware but do nothing visible; the working path is usually the per-key custom-frame engine. Verify hardware behavior on-device rather than trusting a success status.
- **Error handling:** `Result<T, E>` throughout; avoid `unwrap()` in daemon code paths. The GUI installs a panic hook via `setup_panic_hook()`.
- **Power profiles:** index 0 = AC, 1 = Battery in `Configuration.power[]`.
- **Device features:** `["fan", "logo", "boost", "per_key_rgb", "bho", "creator_mode"]` in `laptops.json`. Check feature availability before sending hardware commands.
- **Udev rules:** [data/udev/70-openrazer-hidraw.rules](data/udev/70-openrazer-hidraw.rules) must list every supported device PID.

## Adding a New Device

1. Add an entry to `data/devices/laptops.json` with the correct `vid`, `pid`, `features`, and `fan` range.
2. Add the PID to `data/udev/70-openrazer-hidraw.rules`.
3. Build, then verify each feature on the real hardware.
