---
applyTo: "plugins/kde-widget/**"
description: "Use when editing the KDE Plasma 6 widget. It is a QML-only plasmoid (no C++, no build step) that drives the daemon by shelling out to the razer-cli helper."
---
# KDE Widget Guidelines

A **QML-only Plasma 6 plasmoid** — there is no C++ and no compilation step.

## Files

- `package/metadata.json` — Plasma 6 plugin metadata (`KPackageStructure: Plasma/Applet`, id `io.github.stizzy98.openrazer.plasmoid`).
- `package/contents/ui/main.qml` — the whole widget (compact panel icon + expanded popup).
- `package/contents/config/main.xml` — widget config schema (kcfg).
- `install-plasmoid.sh` — copies `package/` into `~/.local/share/plasma/plasmoids/<id>/` and rebuilds the KDE cache.

## How it talks to the daemon

The widget cannot speak the daemon's bincode socket protocol. It runs commands through a
`Plasma5Support.DataSource` with the `executable` engine:

- **Reads/writes** go through the `razer-cli` helper binary (`razer-cli read …` / `razer-cli write …`).
  `razer-cli` is an internal helper for the widgets — not a user-facing, documented CLI.
- **Opening the app** activates the running GUI over D-Bus (`org.gtk.Application.Activate` on
  `io.github.stizzy98.openrazer`), falling back to launching `razer-settings`.

## QML conventions

- Bare module imports (`import QtQuick`, not `import QtQuick 2.15`) — Plasma 6 / Qt 6.
- Use `Kirigami.Units` for spacing/sizing and `Kirigami.Theme` for colors; no hardcoded pixels or colors (icon sizes excepted).
- Placeholder `"--"` means "no data yet" — check for it before displaying.
