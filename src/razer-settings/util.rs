/// Determine whether the system is running on AC power.
///
/// Checks multiple common power-supply names so it works across different
/// laptop vendors (AC0, ADP0, ADP1, ACAD).
pub fn check_if_running_on_ac_power() -> Option<bool> {
    for name in ["AC0", "ADP0", "ADP1", "ACAD"] {
        let path = format!("/sys/class/power_supply/{}/online", name);
        if let Ok(content) = std::fs::read_to_string(&path) {
            return Some(content.trim() == "1");
        }
    }
    None
}

/// Retrieve CPU model name from /proc/cpuinfo (first core)
pub fn get_cpu_name() -> Option<String> {
    let contents = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    for line in contents.lines() {
        if line.starts_with("model name") {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() > 1 {
                return Some(parts[1].trim().to_string());
            }
        }
    }
    None
}
