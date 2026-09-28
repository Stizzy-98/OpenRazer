use std::collections::HashMap;
use std::fs;

/// Real color tokens read from the user's active Omarchy theme
/// (~/.local/state/omarchy/current/theme/colors.toml). Falls back to the current "vantablack"
/// theme's literal values if Omarchy isn't installed or the file can't be read/parsed, so the app
/// never fails to start over this.
pub struct OmarchyColors {
    pub background: String,
    pub dark_background: String,
    pub lighter_background: String,
    pub foreground: String,
    pub muted: String,
    pub accent: String,
    pub selection: String,
    pub theme_name: Option<String>,
}

impl Default for OmarchyColors {
    fn default() -> Self {
        OmarchyColors {
            background: "#000000".into(),
            dark_background: "#090909".into(),
            lighter_background: "#1a1a1a".into(),
            foreground: "#ffffff".into(),
            muted: "#7a7a7a".into(),
            accent: "#8d8d8d".into(),
            selection: "#1a1a1a".into(),
            theme_name: None,
        }
    }
}

/// Hand-rolled parser for colors.toml's flat `key = "value"` shape - the file has no nesting or
/// arrays in the section this app cares about, so pulling in a `toml` crate dependency for one
/// startup read isn't worth it.
fn parse_flat_toml(contents: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim().to_string();
            let value = value.trim().trim_matches('"').to_string();
            map.insert(key, value);
        }
    }
    map
}

pub fn load() -> OmarchyColors {
    let fallback = OmarchyColors::default();
    let Ok(home) = std::env::var("HOME") else {
        return fallback;
    };
    let theme_dir = format!("{home}/.local/state/omarchy/current/theme");
    let Ok(contents) = fs::read_to_string(format!("{theme_dir}/colors.toml")) else {
        return fallback;
    };
    let tokens = parse_flat_toml(&contents);
    let get = |key: &str, default: &str| {
        tokens
            .get(key)
            .cloned()
            .unwrap_or_else(|| default.to_string())
    };

    let theme_name = fs::read_to_string(format!("{home}/.local/state/omarchy/current/theme.name"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    OmarchyColors {
        background: get("background", &fallback.background),
        dark_background: get("dark_background", &fallback.dark_background),
        lighter_background: get("lighter_background", &fallback.lighter_background),
        foreground: get("foreground", &fallback.foreground),
        muted: get("muted", &fallback.muted),
        accent: get("accent", &fallback.accent),
        selection: get("selection", &fallback.selection),
        theme_name,
    }
}

/// Builds the app's full stylesheet from the given theme tokens. Interaction-state alpha values
/// (idle/hover/focus/selected fill+border) are lifted directly from this same Omarchy theme
/// system's shell.toml [controls] section, not guessed generic hover styles. Font and card/tab
/// treatment follow the real CSS Omarchy ships for its own GTK surfaces
/// (hyprland-preview-share-picker.css) - sharp 5px corners, solid accent borders, underline-style
/// selected tabs instead of Adwaita's default filled pill.
pub fn build_css(c: &OmarchyColors) -> String {
    format!(
        r#"
@define-color accent_color {accent};
@define-color accent_bg_color {accent};
@define-color accent_fg_color {background};
@define-color window_bg_color {background};
@define-color view_bg_color {dark_background};
@define-color card_bg_color {lighter_background};
@define-color window_fg_color {foreground};

selection, *:selected {{
    background-color: {selection};
}}

* {{
    font-family: "JetBrains Mono NF", "JetBrains Mono", monospace;
}}

headerbar, .background {{
    background-color: {background};
    color: {foreground};
}}

headerbar {{
    border-bottom: 1px solid alpha({accent}, 0.4);
}}

headerbar .title {{
    font-weight: bold;
}}

headerbar .subtitle {{
    color: {muted};
    font-weight: normal;
}}

/* Sharp, bordered cards instead of Adwaita's default rounded/borderless boxed-list */
.boxed-list, list.boxed-list, row {{
    background-color: {lighter_background};
}}

.boxed-list {{
    border: 1px solid alpha({accent}, 0.4);
    border-radius: 5px;
}}

row:selected {{
    background-color: alpha({accent}, 0.18);
    border-left: 2px solid {accent};
}}

/* Underline-style selected tab, matching Omarchy's own tab convention rather than a filled pill */
viewswitcherbutton {{
    border-radius: 0;
    border-bottom: 2px solid transparent;
}}

viewswitcherbutton:checked {{
    background: transparent;
    border-bottom: 2px solid {accent};
    box-shadow: none;
}}

/* Idle/hover/focus/selected alphas lifted from shell.toml's [controls] tokens */
button {{
    border: 1px solid alpha({foreground}, 0.4);
    border-radius: 5px;
    background-color: alpha({foreground}, 0.04);
}}

button:hover, button:focus {{
    background-color: alpha({foreground}, 0.08);
    border-color: alpha({foreground}, 0.25);
}}

button.suggested-action {{
    background-color: {accent};
    color: {background};
    border-color: {accent};
}}

button.suggested-action:hover {{
    background-color: alpha({accent}, 0.85);
}}

scale trough {{
    background-color: alpha({foreground}, 0.12);
    border-radius: 3px;
}}

scale trough highlight {{
    background-color: {accent};
    border-radius: 3px;
}}

scale slider {{
    background-color: {foreground};
    border: 1px solid {accent};
    border-radius: 999px;
}}

/* Razer-branded green glow kept off in favor of a plain accent-tinted hover */
button:hover {{
    transition: all 150ms ease;
}}

/* Tabular numbers for aligned power/sensor readings */
.numeric {{
    font-feature-settings: "tnum";
}}

/* System monitor status bar - subtle top border */
.monitor-bar {{
    padding: 4px 0;
    border-top: 1px solid alpha({accent}, 0.4);
}}

/* AC/Battery toggle - slightly bolder when selected */
.linked>button:checked {{
    font-weight: 600;
    background-color: alpha({accent}, 0.18);
}}
"#,
        accent = c.accent,
        background = c.background,
        dark_background = c.dark_background,
        lighter_background = c.lighter_background,
        foreground = c.foreground,
        muted = c.muted,
        selection = c.selection,
    )
}
