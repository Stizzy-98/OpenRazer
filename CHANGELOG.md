# Changelog

## 0.4.0 (2026-09-26)

### New Features

- **Full native Chroma effect suite in the GUI** — Off, Static, Wave, Breathing, Reactive,
  Spectrum, Starlight, driven directly by the daemon's native `SetStandardEffect` protocol path
  instead of the old 4-preset "software effect" combo, which sent the wrong byte layout for
  several effects on this hardware (e.g. Breathing sent `[r,g,b,duration]` into a report that
  expects `[mode,r1,g1,b1,r2,g2,b2]`)
- **Per-key RGB painting** — a click-to-paint 96-key grid (Fill All / Random All / Clear All),
  built on the daemon's per-key custom-frame engine
- **Wheel effect** — a software-driven circular rainbow rotation on the per-key engine (Razer's
  own hardware Wheel effect isn't implemented on this laptop's firmware)
- **Audio Meter** — a real-time, FFT-based reactive spectrum effect (bass/mid/treble), with
  configurable sensitivity, decay, and brightness, and 3 color modes (Rainbow, Static, Intensity
  Gradient)
- **Omarchy theming** — the GUI reads the active Omarchy theme's real color tokens and font at
  startup and matches its look automatically
- **Stars effect** — a timed version of the per-key paint canvas's "Random All" button; every key
  re-rolls to a new independent random color on a fixed interval (1-4 speed setting, 1=slowest/
  1.00s between re-rolls, 4=fastest/0.25s)
- **Ripple effect**: each key press sends a ring of light outward across the keyboard, in a
  static color or a rainbow that shifts through the color wheel with distance, with a 1–4 speed
  slider. Key presses are read from the keyboard's own HID interface (no `input` group needed)
  only while Ripple is active, and only grid positions are kept
- **Per-key lighting on 37 models** (was 2): the per-key grid size now comes from each model's
  device entry (6 × 16 up to 6 × 25, from OpenRazer's device tables), so Wheel, Audio Meter,
  Stars, Ripple, CPU Temperature, and painting work across them. The paint grid is sized to the
  model and hidden on models without per-key lighting
- **CPU Temperature effect**: the whole keyboard shows CPU temperature from blue (cool) to red
  (hot), with adjustable Cool At / Hot At temperatures
- **Ripple random color mode**: each key press gets its own random color
- **Keyboard layout detection**: paint-grid key labels match the keyboard's reported layout
  (US, ISO layouts, QWERTZ, AZERTY), using positions measured on real hardware
- **Device details** on the About page: keyboard firmware version, layout, lighting grid size,
  and BIOS version (plus the keyboard's serial number on models that store one)
- Brightness uses the Blade backlight command (falling back to the generic one) and is read back
  from the keyboard, so changes made with the Fn keys show up in the app
- Lighting is now the first tab
- Launching the app while it's already open brings the existing window to the front instead of
  starting a second copy
- Speed controls for Wheel, Stars, Ripple, Reactive, and Starlight are 1–4 sliders
- After a reinstall or package update, the app and the daemon restart themselves into the new
  version within a few seconds, instead of a stale copy lingering in the tray or answering the
  new app with an older protocol. The app stays hidden if it was closed to the tray

### Changes

- App ID `io.github.stizzy98.openrazer` (desktop entry, icon, GApplication ID); packages are
  named `openrazer`
- The app is GUI-only; `razer-cli` remains as an internal helper for the desktop widgets
- `razer-cli` covers everything the app does: every effect (including Ripple and CPU
  Temperature), per-key painting (`paint key/fill/random/clear`), and new reads for the device
  (`read device`), the active effect (`read effect`), the key colors (`read frame`), and the idle
  timeout (`read`/`write idle`). Arguments are range-checked, failures exit with status 1, and
  the daemon's raw responses are no longer printed
- No donation prompts and no update checkers in the app or the KDE widget
- Removed the unused Blade 16 2025 thermal-safety module and the non-functional C++ KDE applet
- Lighting commands use transaction id 0xFF on every model, as OpenRazer does for all Blade
  laptops (previously only the Blade 15 Advanced Early 2022)
- Device list cross-checked against OpenRazer: removed a duplicate Blade 15 Advanced (Early 2022)
  entry, and dropped the logo toggle on 15 models OpenRazer lists without logo control; a test
  now checks the list for duplicate PIDs, missing udev entries, and invalid grid sizes

### Security

- The keyboard's HID interfaces were world-readable and writable (`MODE="0666"`), letting any
  local account read keystrokes and send raw hardware commands. Access is now limited to the
  logged-in desktop user via a `uaccess` ACL. The rules file is now `70-openrazer-hidraw.rules`
  (it must sort before `73-seat-late.rules` for the ACL to apply); the source installers remove the
  old `99-hidraw-permissions.rules` and reset already-present device nodes
- The daemon socket is created owner-only and never falls back to `/tmp`; config and lock files
  never fall back to `/tmp` either
- The daemon drops clients that stall or send oversized requests instead of blocking
- Removed the OpenRC service, which ran without a user runtime directory (world-writable socket
  in `/tmp`) and launched a binary path that doesn't exist
- The daemon validates client input it forwards to hardware: out-of-range per-key paint indexes no
  longer crash it, and battery charge limits outside 50–80% are rejected before reaching the
  battery controller
- The systemd service runs with kernel-enforced hardening (no privilege gain, no writable+
  executable memory, restricted namespaces/realtime/personality, private file permissions)
- Release artifacts ship with a `SHA256SUMS` file, and CI actions are pinned to commit SHAs
- Removed redundant `unsafe impl Send/Sync` on the effect engine so the compiler verifies its
  thread safety
- Dropped the unused `systemstat` dependency (which pulled in `time` with RUSTSEC-2026-0009) and
  updated yanked crates

### Bug Fixes

- Fix `razer-cli standard-effect breathing`/`starlight` always sending both colors: Single now
  sends one color and Random none, as the firmware expects
- Fix the KDE widget never showing Battery Health Optimizer as off (the CLI printed it to stderr)

- Fix Static colors not applying: the native Static effect is acknowledged but ignored by this
  firmware, so Static now renders through the per-key engine
- Fix the HID packet checksum, which covered the wrong byte range and was sent as 0x00 on the
  first attempt of every command
- Fix dragging the fan slider while Auto was on snapping the fan back to its minimum RPM
- Fix the Stars, Software Breathing, and Audio Meter effects disappearing after a daemon restart
- Fix the logo LED command to match the reference Blade implementation (state command,
  transaction ID 0xFF); note the Blade 15 Advanced Early 2022 firmware still rejects it

- Fix Wave direction (wire values are 1/2, not 0/1) and its GUI labels being swapped
- Fix Reactive speed (wire range is 1-4, not 0-255)
- Fix Breathing/Starlight color-mode byte (wire values are 1/2/3 for Single/Dual/Random, not
  0-indexed)
- Fix per-effect `data_size` header (was hardcoded to 80 for every effect instead of the real
  fixed per-effect-type constant) and the transaction ID for this device (0xFF, not the daemon's
  default 0x1F)
- Fix the custom-frame HID report's RGB payload offset (was `args[7]`, should be `args[4]`) and
  packet size, which made per-key painting a no-op even when the feature was enabled
- Fix the board model being 15 columns instead of 16 (per OpenRazer's own `MATRIX_DIMS` for this
  device), which silently dropped an entire column of keys (Power, Backspace, `\`, Enter, Shift,
  Down arrow all landed in the missing column)
- Fix the per-key effect/layer mask system being hardcoded to 90 keys instead of 96
