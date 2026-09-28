# Razer Control — KDE Plasma Widget

A Plasma 6 panel widget for Razer Control. Pure QML — nothing to compile.

## What it does

Click the panel icon to open a popup with:

- **Live system monitor** — CPU/iGPU/dGPU temperatures, frequencies, power draw (including CPU
  package power via RAPL), and utilization
- **Battery status** — charge level and charge/discharge wattage
- **Quick toggles** — click to cycle Power Profile, Fan, Keyboard Brightness, Logo, and Charge Limit
  for the current power source (AC or battery)
- **Open the full app** — click the header to bring up Razer Settings

## Requirements

- KDE Plasma 6.0+
- Razer Control installed and its daemon running

## Install

```bash
./install-plasmoid.sh
```

This copies `package/` to `~/.local/share/plasma/plasmoids/io.github.stizzy98.openrazer.plasmoid/`
and rebuilds the KDE cache. Then right-click your panel → **Add Widgets** → search
**"Razer Control"**.

## Uninstall

```bash
rm -rf ~/.local/share/plasma/plasmoids/io.github.stizzy98.openrazer.plasmoid
kbuildsycoca6
```

## Troubleshooting

- **Widget missing from the list:** run `kbuildsycoca6`, then restart Plasma with
  `plasmashell --replace &`.
- **"Unsupported Widget":** you're on Plasma 5; this widget needs Plasma 6.
- **Toggles do nothing:** make sure the daemon is running (`systemctl --user status razercontrol`).

## Files

- `package/metadata.json` — Plasma 6 plugin metadata
- `package/contents/ui/main.qml` — the widget
- `package/contents/config/main.xml` — widget settings schema
