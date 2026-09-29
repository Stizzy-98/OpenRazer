//! Types and IPC shared by the daemon, the GUI, and the widget helper binary.

pub mod comms;

use serde::{Deserialize, Serialize};

const DEVICE_FILE_DEFAULT: &str = "/usr/share/razercontrol/laptops.json";

/// Battery Health Optimizer charge caps the hardware accepts: multiples of 5 from 50 to 80.
pub fn valid_bho_threshold(threshold: u8) -> bool {
    threshold.is_multiple_of(5) && (50..=80).contains(&threshold)
}

pub fn device_file_path() -> String {
    std::env::var("RAZER_DEVICE_FILE").unwrap_or_else(|_| DEVICE_FILE_DEFAULT.to_string())
}

/// Notices when this process's own executable is replaced on disk by a reinstall or package
/// update, so a long-running binary can restart into the new version instead of running stale
/// code against newer components (the daemon/GUI socket protocol changes between versions).
///
/// Installers and package managers write the new file under a fresh inode, which the kernel
/// reports by appending " (deleted)" to `/proc/self/exe`. Call `poll` periodically: it returns
/// the new executable's path once one is in place and has been unchanged across two consecutive
/// polls, so a restart never runs a half-written file.
#[derive(Default)]
pub struct UpdateWatch {
    last_seen: Option<(u64, std::time::SystemTime)>,
}

impl UpdateWatch {
    pub fn poll(&mut self) -> Option<std::path::PathBuf> {
        let exe = std::fs::read_link("/proc/self/exe").ok()?;
        let path = std::path::PathBuf::from(exe.to_str()?.strip_suffix(" (deleted)")?);
        let meta = std::fs::metadata(&path).ok()?;
        let seen = (meta.len(), meta.modified().ok()?);
        if self.last_seen.replace(seen) == Some(seen) {
            Some(path)
        } else {
            None
        }
    }
}

/// Widest per-key lighting grid of any supported model (Blade Pro 2017: 6 x 25).
pub const MAX_MATRIX_COLS: usize = 25;
/// Every supported model's per-key grid has 6 rows.
pub const MATRIX_ROWS: usize = 6;
const DEFAULT_MATRIX_COLS: usize = 16;

/// Reads the CPU package temperature in °C (AMD k10temp/zenpower, Intel coretemp, else the first
/// thermal zones).
pub fn cpu_temperature() -> Option<f64> {
    let read = |path: std::path::PathBuf| -> Option<f64> {
        Some(
            std::fs::read_to_string(path)
                .ok()?
                .trim()
                .parse::<f64>()
                .ok()?
                / 1000.0,
        )
    };
    for entry in std::fs::read_dir("/sys/class/hwmon").ok()?.flatten() {
        let name = std::fs::read_to_string(entry.path().join("name")).unwrap_or_default();
        if matches!(name.trim(), "k10temp" | "zenpower" | "coretemp")
            && let Some(t) = read(entry.path().join("temp1_input"))
        {
            return Some(t);
        }
    }
    (0..3).find_map(|z| read(format!("/sys/class/thermal/thermal_zone{z}/temp").into()))
}

/// Physical keyboard layouts the keyboard reports, by id (OpenRazer's table). Others exist but
/// aren't known.
pub fn keyboard_layout_name(id: u8) -> Option<&'static str> {
    Some(match id {
        0x01 => "US English",
        0x02 => "Greek",
        0x03 => "German",
        0x04 => "French",
        0x05 => "Russian",
        0x06 => "UK English",
        0x07 => "Nordic",
        0x0A => "Turkish",
        0x0C => "Japanese",
        0x0F => "Swiss German",
        0x10 => "Spanish",
        0x11 => "Italian",
        0x12 => "Portuguese",
        0x81 => "US English (Mac)",
        _ => return None,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupportedDevice {
    pub name: String,
    pub vid: String,
    pub pid: String,
    pub features: Vec<String>,
    pub fan: Vec<u16>,
    /// Per-key lighting grid as [rows, columns], for models with `per_key_rgb`.
    #[serde(default)]
    pub matrix: Option<[usize; 2]>,
}

impl SupportedDevice {
    /// Columns in the per-key lighting grid.
    pub fn matrix_cols(&self) -> usize {
        self.matrix
            .map_or(DEFAULT_MATRIX_COLS, |[_, cols]| cols)
            .clamp(1, MAX_MATRIX_COLS)
    }

    pub fn has_feature(&self, feature: &str) -> bool {
        self.features.iter().any(|f| f == feature)
    }

    pub fn can_boost(&self) -> bool {
        self.has_feature("boost")
    }

    pub fn has_logo(&self) -> bool {
        self.has_feature("logo")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAPTOPS: &str = include_str!("../data/devices/laptops.json");
    const UDEV_RULES: &str = include_str!("../data/udev/70-openrazer-hidraw.rules");

    /// Every model must be reachable (unique PID, listed in the udev rule) and every per-key model
    /// needs a grid the engine can drive. Grid sizes and per-key support were taken from
    /// OpenRazer's device tables.
    #[test]
    fn device_list_is_consistent() {
        let devices: Vec<SupportedDevice> = serde_json::from_str(LAPTOPS).unwrap();
        let mut pids: Vec<String> = devices.iter().map(|d| d.pid.to_lowercase()).collect();
        for pid in &pids {
            assert!(
                UDEV_RULES.contains(pid.as_str()),
                "PID {pid} missing from udev rules"
            );
        }
        pids.sort();
        let count = pids.len();
        pids.dedup();
        assert_eq!(count, pids.len(), "duplicate PID in laptops.json");

        for d in &devices {
            match (d.has_feature("per_key_rgb"), d.matrix) {
                (true, Some([rows, cols])) => {
                    assert_eq!(rows, MATRIX_ROWS, "{}", d.name);
                    assert!((1..=MAX_MATRIX_COLS).contains(&cols), "{}", d.name);
                }
                (true, None) => panic!("{} has per_key_rgb but no matrix", d.name),
                (false, Some(_)) => panic!("{} has a matrix but no per_key_rgb", d.name),
                (false, None) => {}
            }
        }
    }

    #[test]
    fn bho_threshold_accepts_only_the_ui_range() {
        for t in [50, 55, 60, 65, 70, 75, 80] {
            assert!(valid_bho_threshold(t), "{t} should be accepted");
        }
        for t in [0, 10, 45, 49, 51, 79, 85, 100, 128, 200, 255] {
            assert!(!valid_bho_threshold(t), "{t} should be rejected");
        }
    }
}
