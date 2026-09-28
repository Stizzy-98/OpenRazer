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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupportedDevice {
    pub name: String,
    pub vid: String,
    pub pid: String,
    pub features: Vec<String>,
    pub fan: Vec<u16>,
}

impl SupportedDevice {
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
    use super::valid_bho_threshold;

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
