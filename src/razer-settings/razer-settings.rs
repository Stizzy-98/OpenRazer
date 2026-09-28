use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;
use std::cell::Cell;
use std::cell::RefCell;
use std::fs;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use service::comms;
mod error_handling;
mod gpu_monitor;
mod omarchy_theme;
mod tray;
mod util;
mod widgets;

use error_handling::*;
use service::SupportedDevice;
use util::*;
use widgets::*;

/// Result of trying to become the one running `razer-settings` instance.
enum InstanceLock {
    /// Lock acquired - this is the only instance. Keep the `File` alive for the process's
    /// entire lifetime; the flock releases automatically when it (or the process) is dropped.
    Acquired(fs::File),
    /// The lock is already held by another live process - a real duplicate. Carries that
    /// process's PID (read from the lock file's contents) if it was parseable, so the caller
    /// can ask it to raise its window instead of just exiting silently.
    AlreadyRunning(Option<i32>),
    /// Couldn't even test the lock (e.g. no writable HOME) - fail open rather than block
    /// startup over an unrelated filesystem problem.
    Unavailable,
}

/// Enforces a single running `razer-settings` instance at the OS process level, independent of
/// GApplication's own D-Bus-based uniqueness (which is already configured via `application_id`
/// below, but has been observed letting duplicate processes pile up rather than handing off to
/// the existing one - repeated launches left several full GTK processes running at once instead
/// of focusing one window). An exclusive advisory lock on a fixed file is held for the process's
/// entire lifetime (it releases automatically on exit, including a crash, since the kernel drops
/// flock locks when the holding fd closes) - a second launch fails to acquire it immediately and
/// exits before touching GTK/D-Bus at all, so at most one instance can ever be running. The
/// lock-holder writes its own PID into the file so a rejected duplicate can signal it (see
/// `FOCUS_SIGNAL` below) to raise its window instead of the duplicate opening a new one.
fn acquire_single_instance_lock() -> InstanceLock {
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::os::unix::io::AsRawFd;

    // Never fall back to a shared directory: a lock file another user could plant there would
    // make this process signal an arbitrary PID (see FOCUS_SIGNAL).
    let Some(home) = std::env::home_dir() else {
        return InstanceLock::Unavailable;
    };
    let dir = format!("{}/.local/share/razercontrol", home.display());
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("Warning: couldn't create {dir} ({e}), skipping single-instance lock");
        return InstanceLock::Unavailable;
    }
    let lock_path = format!("{dir}/razer-settings.lock");
    // Never truncate on open - a duplicate's open must not destroy the PID the real holder
    // already wrote; only the process that actually wins the flock below rewrites it.
    let mut file = match fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Warning: couldn't open {lock_path} ({e}), skipping single-instance lock");
            return InstanceLock::Unavailable;
        }
    };
    // SAFETY: flock() only touches the kernel's lock table for this fd; no aliasing/lifetime
    // concerns beyond the fd itself, which `file` owns for as long as the returned value lives.
    let ret = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if ret == 0 {
        let _ = file.set_len(0);
        let _ = file.seek(SeekFrom::Start(0));
        let _ = write!(file, "{}", std::process::id());
        let _ = file.flush();
        InstanceLock::Acquired(file)
    } else {
        let mut contents = String::new();
        let _ = file.read_to_string(&mut contents);
        InstanceLock::AlreadyRunning(contents.trim().parse::<i32>().ok())
    }
}

/// Sent to the running instance to ask it to raise its window; picked for having no other
/// meaning anywhere else in this codebase (no existing signal handling in this binary).
const FOCUS_SIGNAL: i32 = libc::SIGUSR1;

/// Set when restarting after an update from a window that was hidden in the tray.
const RESTART_HIDDEN_ENV: &str = "RAZER_SETTINGS_RESTART_HIDDEN";

/// Replaces this process with `exe` (same PID). The single-instance lock and every other fd are
/// close-on-exec, so the new image starts as a normal first launch. Only returns if exec fails,
/// in which case this instance just keeps running.
fn restart_into(exe: &std::path::Path, window_visible: bool) {
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new(exe);
    if window_visible {
        cmd.env_remove(RESTART_HIDDEN_ENV);
    } else {
        cmd.env(RESTART_HIDDEN_ENV, "1");
    }
    let err = cmd.exec();
    eprintln!("Couldn't restart into the updated {}: {err}", exe.display());
}

fn send_data(opt: comms::DaemonCommand) -> Option<comms::DaemonResponse> {
    match comms::try_bind() {
        Ok(socket) => comms::send_to_daemon(opt, socket),
        Err(error) => {
            eprintln!("Can't connect to daemon: {}", error);
            None
        }
    }
}

fn get_gpu_status() -> Option<(Vec<comms::GpuInfo>, bool, String, bool)> {
    let response = send_data(comms::DaemonCommand::GetGpuStatus)?;
    use comms::DaemonResponse::*;
    match response {
        GetGpuStatus {
            gpus,
            dgpu_runtime_pm,
            envycontrol_mode,
            envycontrol_available,
        } => Some((
            gpus,
            dgpu_runtime_pm,
            envycontrol_mode,
            envycontrol_available,
        )),
        response => {
            println!("Instead of GetGpuStatus got {response:?}");
            None
        }
    }
}

fn set_dgpu_runtime_pm(enabled: bool) -> Option<bool> {
    let response = send_data(comms::DaemonCommand::SetDgpuRuntimePM { enabled })?;
    use comms::DaemonResponse::*;
    match response {
        SetDgpuRuntimePM { result } => Some(result),
        response => {
            println!("Instead of SetDgpuRuntimePM got {response:?}");
            None
        }
    }
}

fn set_gpu_mode(mode: &str) -> Option<(bool, String)> {
    let response = send_data(comms::DaemonCommand::SetGpuMode {
        mode: mode.to_string(),
    })?;
    use comms::DaemonResponse::*;
    match response {
        SetGpuMode { result, message } => Some((result, message)),
        response => {
            println!("Instead of SetGpuMode got {response:?}");
            None
        }
    }
}

fn get_device_name() -> Option<String> {
    let response = send_data(comms::DaemonCommand::GetDeviceName)?;
    use comms::DaemonResponse::*;
    match response {
        GetDeviceName { name } => Some(name),
        response => {
            println!("Instead of GetDeviceName got {response:?}");
            None
        }
    }
}

/// Show an error dialog to the user without panicking.
/// This is safe to call from GTK signal callbacks (no panic/unwind).
fn show_error_dialog(app: &adw::Application, message: &str) {
    let dialog = adw::MessageDialog::new(
        app.active_window().as_ref(),
        Some("Razer Control — Error"),
        Some(message),
    );
    dialog.add_response("close", "Close");
    dialog.set_default_response(Some("close"));
    dialog.set_close_response("close");
    dialog.connect_response(None, |dlg, _| {
        dlg.close();
    });
    dialog.present();
}

fn get_bho() -> Option<(bool, u8)> {
    let response = send_data(comms::DaemonCommand::GetBatteryHealthOptimizer())?;
    use comms::DaemonResponse::*;
    match response {
        GetBatteryHealthOptimizer { is_on, threshold } => Some((is_on, threshold)),
        response => {
            println!("Instead of GetBatteryHealthOptimizer got {response:?}");
            None
        }
    }
}

fn set_bho(is_on: bool, threshold: u8) -> Option<bool> {
    let response = send_data(comms::DaemonCommand::SetBatteryHealthOptimizer { is_on, threshold })?;
    use comms::DaemonResponse::*;
    match response {
        SetBatteryHealthOptimizer { result } => Some(result),
        response => {
            println!("Instead of SetBatteryHealthOptimizer got {response:?}");
            None
        }
    }
}

fn get_brightness(ac: bool) -> Option<u8> {
    let ac = if ac { 1 } else { 0 };
    let response = send_data(comms::DaemonCommand::GetBrightness { ac })?;
    use comms::DaemonResponse::*;
    match response {
        GetBrightness { result } => Some(result),
        response => {
            println!("Instead of GetBrightness got {response:?}");
            None
        }
    }
}

fn set_brightness(ac: bool, val: u8) -> Option<bool> {
    let ac = if ac { 1 } else { 0 };
    let response = send_data(comms::DaemonCommand::SetBrightness { ac, val })?;
    use comms::DaemonResponse::*;
    match response {
        SetBrightness { result } => Some(result),
        response => {
            println!("Instead of SetBrightness got {response:?}");
            None
        }
    }
}

fn get_logo(ac: bool) -> Option<u8> {
    let ac = if ac { 1 } else { 0 };
    let response = send_data(comms::DaemonCommand::GetLogoLedState { ac })?;
    use comms::DaemonResponse::*;
    match response {
        GetLogoLedState { logo_state } => Some(logo_state),
        response => {
            println!("Instead of GetLogoLedState got {response:?}");
            None
        }
    }
}

fn set_logo(ac: bool, logo_state: u8) -> Option<bool> {
    let ac = if ac { 1 } else { 0 };
    let response = send_data(comms::DaemonCommand::SetLogoLedState { ac, logo_state })?;
    use comms::DaemonResponse::*;
    match response {
        SetLogoLedState { result } => Some(result),
        response => {
            println!("Instead of SetLogoLedState got {response:?}");
            None
        }
    }
}

fn get_standard_effect() -> Option<(u8, Vec<u8>)> {
    let response = send_data(comms::DaemonCommand::GetStandardEffect)?;
    use comms::DaemonResponse::*;
    match response {
        GetStandardEffect { effect, params } => Some((effect, params)),
        response => {
            println!("Instead of GetStandardEffect got {response:?}");
            None
        }
    }
}

fn set_standard_effect(name: &str, params: Vec<u8>) -> Option<bool> {
    let response = send_data(comms::DaemonCommand::SetStandardEffect {
        name: name.into(),
        params,
    })?;
    use comms::DaemonResponse::*;
    match response {
        SetStandardEffect { result } => Some(result),
        response => {
            println!("Instead of SetStandardEffect got {response:?}");
            None
        }
    }
}

fn get_keyboard_rgb() -> Option<Vec<u8>> {
    let response = send_data(comms::DaemonCommand::GetKeyboardRGB { layer: -1 })?;
    use comms::DaemonResponse::*;
    match response {
        GetKeyboardRGB { rgbdata, .. } => Some(rgbdata),
        response => {
            println!("Instead of GetKeyboardRGB got {response:?}");
            None
        }
    }
}

fn set_custom_key(index: u8, r: u8, g: u8, b: u8) -> Option<bool> {
    let response = send_data(comms::DaemonCommand::SetCustomKey { index, r, g, b })?;
    use comms::DaemonResponse::*;
    match response {
        SetCustomKey { result } => Some(result),
        response => {
            println!("Instead of SetCustomKey got {response:?}");
            None
        }
    }
}

fn fill_custom_frame(r: u8, g: u8, b: u8) -> Option<bool> {
    let response = send_data(comms::DaemonCommand::FillCustomFrame { r, g, b })?;
    use comms::DaemonResponse::*;
    match response {
        FillCustomFrame { result } => Some(result),
        response => {
            println!("Instead of FillCustomFrame got {response:?}");
            None
        }
    }
}

fn randomize_custom_frame() -> Option<bool> {
    let response = send_data(comms::DaemonCommand::RandomizeCustomFrame)?;
    use comms::DaemonResponse::*;
    match response {
        RandomizeCustomFrame { result } => Some(result),
        response => {
            println!("Instead of RandomizeCustomFrame got {response:?}");
            None
        }
    }
}

fn set_wheel_effect(direction: u8, speed: u8) -> Option<bool> {
    let response = send_data(comms::DaemonCommand::SetWheelEffect { direction, speed })?;
    use comms::DaemonResponse::*;
    match response {
        SetWheelEffect { result } => Some(result),
        response => {
            println!("Instead of SetWheelEffect got {response:?}");
            None
        }
    }
}

fn set_sound_bar_effect(
    color_mode: u8,
    r: u8,
    g: u8,
    b: u8,
    sensitivity: u8,
    decay: u8,
    brightness: u8,
) -> Option<bool> {
    let response = send_data(comms::DaemonCommand::SetSoundBarEffect {
        color_mode,
        r,
        g,
        b,
        sensitivity,
        decay,
        brightness,
    })?;
    use comms::DaemonResponse::*;
    match response {
        SetSoundBarEffect { result } => Some(result),
        response => {
            println!("Instead of SetSoundBarEffect got {response:?}");
            None
        }
    }
}

fn set_stars_effect(speed: u8) -> Option<bool> {
    let response = send_data(comms::DaemonCommand::SetStarsEffect { speed })?;
    use comms::DaemonResponse::*;
    match response {
        SetStarsEffect { result } => Some(result),
        response => {
            println!("Instead of SetStarsEffect got {response:?}");
            None
        }
    }
}

fn set_ripple_effect(color_mode: u8, r: u8, g: u8, b: u8, speed: u8) -> Option<bool> {
    let response = send_data(comms::DaemonCommand::SetRippleEffect {
        color_mode,
        r,
        g,
        b,
        speed,
    })?;
    use comms::DaemonResponse::*;
    match response {
        SetRippleEffect { result } => Some(result),
        response => {
            println!("Instead of SetRippleEffect got {response:?}");
            None
        }
    }
}

fn get_power(ac: bool) -> Option<(u8, u8, u8)> {
    let ac = if ac { 1 } else { 0 };
    let mut result = (0, 0, 0);

    let response = send_data(comms::DaemonCommand::GetPwrLevel { ac })?;
    use comms::DaemonResponse::*;
    match response {
        GetPwrLevel { pwr } => result.0 = pwr,
        response => {
            println!("Instead of GetPwrLevel got {response:?}");
            return None;
        }
    }

    let response = send_data(comms::DaemonCommand::GetCPUBoost { ac })?;
    match response {
        GetCPUBoost { cpu } => result.1 = cpu,
        response => {
            println!("Instead of GetCPUBoost got {response:?}");
            return None;
        }
    }

    let response = send_data(comms::DaemonCommand::GetGPUBoost { ac })?;
    match response {
        GetGPUBoost { gpu } => result.2 = gpu,
        response => {
            println!("Instead of GetGPUBoost got {response:?}");
            return None;
        }
    }
    Some(result)
}

fn set_power(ac: bool, power: (u8, u8, u8)) -> Option<bool> {
    let ac = if ac { 1 } else { 0 };
    let response = send_data(comms::DaemonCommand::SetPowerMode {
        ac,
        pwr: power.0,
        cpu: power.1,
        gpu: power.2,
    })?;
    use comms::DaemonResponse::*;
    match response {
        SetPowerMode { result } => Some(result),
        response => {
            println!("Instead of SetPowerMode got {response:?}");
            None
        }
    }
}

fn get_fan_speed(ac: bool) -> Option<i32> {
    let ac = if ac { 1 } else { 0 };
    let response = send_data(comms::DaemonCommand::GetFanSpeed { ac })?;
    use comms::DaemonResponse::*;
    match response {
        GetFanSpeed { rpm } => Some(rpm),
        response => {
            println!("Instead of GetFanSpeed got {response:?}");
            None
        }
    }
}

fn set_fan_speed(ac: bool, value: i32) -> Option<bool> {
    let ac = if ac { 1 } else { 0 };
    let response = send_data(comms::DaemonCommand::SetFanSpeed { ac, rpm: value })?;
    use comms::DaemonResponse::*;
    match response {
        SetFanSpeed { result } => Some(result),
        response => {
            println!("Instead of SetFanSpeed got {response:?}");
            None
        }
    }
}

/// Read CPU temperature from hwmon (supports AMD k10temp/zenpower and Intel coretemp)
fn get_cpu_temperature() -> Option<f64> {
    if let Ok(entries) = fs::read_dir("/sys/class/hwmon") {
        for entry in entries.flatten() {
            let name_path = entry.path().join("name");
            if let Ok(name) = fs::read_to_string(&name_path) {
                let name = name.trim();
                if name == "k10temp" || name == "zenpower" || name == "coretemp" {
                    let temp_path = entry.path().join("temp1_input");
                    if let Ok(content) = fs::read_to_string(&temp_path)
                        && let Ok(temp) = content.trim().parse::<f64>()
                    {
                        return Some(temp / 1000.0);
                    }
                }
            }
        }
    }

    let paths = [
        "/sys/class/thermal/thermal_zone0/temp",
        "/sys/class/thermal/thermal_zone1/temp",
        "/sys/class/thermal/thermal_zone2/temp",
    ];

    for path in paths {
        if let Ok(content) = fs::read_to_string(path)
            && let Ok(temp) = content.trim().parse::<f64>()
        {
            return Some(temp / 1000.0);
        }
    }
    None
}

/// Read system/CPU power consumption from RAPL (supports AMD and Intel)
fn get_system_power() -> Option<f64> {
    let energy_paths = [
        "/sys/class/powercap/amd-rapl:0/energy_uj",
        "/sys/class/powercap/amd_rapl/amd-rapl:0/energy_uj",
        "/sys/class/powercap/intel-rapl:0/energy_uj",
        "/sys/class/powercap/intel-rapl/intel-rapl:0/energy_uj",
    ];

    for path in &energy_paths {
        if let Ok(content) = fs::read_to_string(path)
            && let Ok(energy) = content.trim().parse::<u64>()
        {
            static LAST_ENERGY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            static LAST_TIME: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_micros() as u64;

            let prev_energy = LAST_ENERGY.swap(energy, std::sync::atomic::Ordering::Relaxed);
            let prev_time = LAST_TIME.swap(now, std::sync::atomic::Ordering::Relaxed);

            if prev_energy > 0 && prev_time > 0 && energy > prev_energy {
                let delta_energy = energy - prev_energy;
                let delta_time = now - prev_time;
                if delta_time > 0 {
                    let power = delta_energy as f64 / delta_time as f64;
                    return Some(power);
                }
            }
            return None; // Found path but need second reading
        }
    }
    None
}

/// Read iGPU power from hwmon (AMD amdgpu) or Intel RAPL GT fallback
fn get_igpu_power() -> Option<f64> {
    if let Ok(entries) = fs::read_dir("/sys/class/hwmon") {
        for entry in entries.flatten() {
            let name_path = entry.path().join("name");
            if let Ok(name) = fs::read_to_string(&name_path)
                && name.trim() == "amdgpu"
            {
                let power_path = entry.path().join("power1_average");
                if let Ok(content) = fs::read_to_string(&power_path)
                    && let Ok(power_uw) = content.trim().parse::<f64>()
                {
                    return Some(power_uw / 1_000_000.0);
                }
            }
        }
    }

    // Fallback: Intel RAPL GT domain
    let paths = [
        "/sys/class/powercap/intel-rapl:0:1/energy_uj",
        "/sys/class/powercap/intel-rapl/intel-rapl:0/intel-rapl:0:1/energy_uj",
    ];

    for path in &paths {
        if let Ok(content) = fs::read_to_string(path)
            && let Ok(energy) = content.trim().parse::<u64>()
        {
            static LAST_IGPU_ENERGY: std::sync::atomic::AtomicU64 =
                std::sync::atomic::AtomicU64::new(0);
            static LAST_IGPU_TIME: std::sync::atomic::AtomicU64 =
                std::sync::atomic::AtomicU64::new(0);

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_micros() as u64;

            let prev_energy = LAST_IGPU_ENERGY.swap(energy, std::sync::atomic::Ordering::Relaxed);
            let prev_time = LAST_IGPU_TIME.swap(now, std::sync::atomic::Ordering::Relaxed);

            if prev_energy > 0 && prev_time > 0 && energy > prev_energy {
                let delta_energy = energy - prev_energy;
                let delta_time = now - prev_time;
                if delta_time > 0 {
                    return Some(delta_energy as f64 / delta_time as f64);
                }
            }
        }
    }
    None
}

/// Read iGPU utilization (AMD gpu_busy_percent or Intel freq-based fallback)
fn get_igpu_utilization() -> Option<u32> {
    for card in ["card0", "card1", "card2"] {
        let busy_path = format!("/sys/class/drm/{}/device/gpu_busy_percent", card);
        if let Ok(content) = fs::read_to_string(&busy_path)
            && let Ok(util) = content.trim().parse::<u32>()
        {
            let driver_path = format!("/sys/class/drm/{}/device/driver", card);
            if let Ok(driver_link) = fs::read_link(&driver_path)
                && driver_link.to_string_lossy().contains("amdgpu")
            {
                return Some(util);
            }
        }
    }

    // Fallback: frequency-based estimation for Intel
    let paths = [
        "/sys/class/drm/card0/gt/gt0/rps_act_freq_mhz",
        "/sys/class/drm/card1/gt/gt0/rps_act_freq_mhz",
    ];
    let max_paths = [
        "/sys/class/drm/card0/gt/gt0/rps_max_freq_mhz",
        "/sys/class/drm/card1/gt/gt0/rps_max_freq_mhz",
    ];

    for (i, path) in paths.iter().enumerate() {
        if let Ok(act_content) = fs::read_to_string(path)
            && let Ok(max_content) = fs::read_to_string(max_paths[i])
            && let (Ok(act), Ok(max)) = (
                act_content.trim().parse::<f64>(),
                max_content.trim().parse::<f64>(),
            )
            && max > 0.0
        {
            return Some(((act / max) * 100.0) as u32);
        }
    }
    None
}

/// Read iGPU temperature from amdgpu hwmon
fn get_igpu_temperature() -> Option<f64> {
    if let Ok(entries) = fs::read_dir("/sys/class/hwmon") {
        for entry in entries.flatten() {
            let name_path = entry.path().join("name");
            if let Ok(name) = fs::read_to_string(&name_path)
                && name.trim() == "amdgpu"
            {
                for temp_file in ["temp1_input", "temp2_input"] {
                    let temp_path = entry.path().join(temp_file);
                    if let Ok(content) = fs::read_to_string(&temp_path)
                        && let Ok(temp) = content.trim().parse::<f64>()
                    {
                        return Some(temp / 1000.0);
                    }
                }
            }
        }
    }
    None
}

/// Read battery percentage from /sys/class/power_supply/BAT{0,1}/capacity
fn get_battery_percentage() -> Option<u8> {
    for bat in ["BAT0", "BAT1"] {
        let path = format!("/sys/class/power_supply/{}/capacity", bat);
        if let Ok(content) = fs::read_to_string(&path)
            && let Ok(pct) = content.trim().parse::<u8>()
        {
            return Some(pct);
        }
    }
    None
}

/// Read battery status (Charging, Discharging, Full, Not charging)
fn get_battery_status() -> Option<String> {
    for bat in ["BAT0", "BAT1"] {
        let path = format!("/sys/class/power_supply/{}/status", bat);
        if let Ok(content) = fs::read_to_string(&path) {
            let status = content.trim().to_string();
            if !status.is_empty() {
                return Some(status);
            }
        }
    }
    None
}

/// Read battery power draw in watts (current_now * voltage_now)
fn get_battery_power() -> Option<f64> {
    for bat in ["BAT0", "BAT1"] {
        let current_path = format!("/sys/class/power_supply/{}/current_now", bat);
        let voltage_path = format!("/sys/class/power_supply/{}/voltage_now", bat);
        if let (Ok(c_str), Ok(v_str)) = (
            fs::read_to_string(&current_path),
            fs::read_to_string(&voltage_path),
        ) && let (Ok(current_ua), Ok(voltage_uv)) =
            (c_str.trim().parse::<u64>(), v_str.trim().parse::<u64>())
            && current_ua > 0
        {
            return Some(current_ua as f64 * voltage_uv as f64 / 1e12);
        }
    }
    None
}

/// Read CPU utilization from /proc/stat (delta-based)
fn get_cpu_utilization() -> Option<u32> {
    static LAST_IDLE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    static LAST_TOTAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    if let Ok(content) = fs::read_to_string("/proc/stat")
        && let Some(line) = content.lines().next()
    {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() >= 5 && fields[0] == "cpu" {
            let mut total: u64 = 0;
            for f in &fields[1..] {
                if let Ok(v) = f.parse::<u64>() {
                    total += v;
                }
            }
            let idle = fields[4].parse::<u64>().unwrap_or(0);

            let prev_idle = LAST_IDLE.swap(idle, std::sync::atomic::Ordering::Relaxed);
            let prev_total = LAST_TOTAL.swap(total, std::sync::atomic::Ordering::Relaxed);

            if prev_total > 0 {
                let d_idle = idle.wrapping_sub(prev_idle);
                let d_total = total.wrapping_sub(prev_total);
                if d_total > 0 {
                    let usage = 100.0 * (1.0 - d_idle as f64 / d_total as f64);
                    return Some(usage.round() as u32);
                }
            }
        }
    }
    None
}

/// Create system monitor panel at the bottom (widget-style layout)
fn create_system_monitor(shared_state: tray::SharedSensorState) -> gtk::Box {
    let main_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    main_box.set_margin_start(16);
    main_box.set_margin_end(16);
    main_box.set_margin_top(4);
    main_box.set_margin_bottom(4);
    main_box.add_css_class("toolbar");
    main_box.add_css_class("monitor-bar");

    // Helper: create a full-width monitor row (name + temp on left, power · util% on right)
    fn make_row(label_text: &str) -> (gtk::Box, gtk::Label, gtk::Label, gtk::Label, gtk::Label) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.set_margin_top(1);
        row.set_margin_bottom(1);

        let name = gtk::Label::new(Some(label_text));
        name.add_css_class("caption");
        name.set_xalign(0.0);
        name.set_opacity(0.6);
        row.append(&name);

        let temp = gtk::Label::new(None);
        temp.add_css_class("caption");
        temp.add_css_class("numeric");
        row.append(&temp);

        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        row.append(&spacer);

        let power = gtk::Label::new(None);
        power.add_css_class("caption");
        power.add_css_class("numeric");
        power.set_opacity(0.7);
        row.append(&power);

        let dot = gtk::Label::new(Some("\u{00B7}"));
        dot.add_css_class("caption");
        dot.set_opacity(0.3);
        row.append(&dot);

        let util = gtk::Label::new(None);
        util.add_css_class("caption");
        util.add_css_class("numeric");
        util.set_opacity(0.7);
        row.append(&util);

        (row, temp, power, dot, util)
    }

    let cpu_name = util::get_cpu_name().unwrap_or_else(|| "CPU".to_string());
    // Shorten extremely long CPU names (e.g. "AMD Ryzen 9 7945HX with Radeon Graphics" -> "AMD Ryzen 9 7945HX")
    let cpu_label = cpu_name
        .replace(" with Radeon Graphics", "")
        .replace(" 16-Core Processor", "");

    // Fetch detected GPUs to find names
    let mut igpu_label = "iGPU".to_string();
    let mut dgpu_label = "dGPU".to_string();

    if let Some((gpu_list, _, _, _)) = get_gpu_status() {
        for gpu_info in gpu_list {
            let name = gpu_info.name;
            // Heuristic: NVIDIA/Discrete usually dGPU; AMD/Intel usually iGPU (unless discrete)
            if name.to_uppercase().contains("NVIDIA") {
                dgpu_label = name.replace(" Laptop GPU", "");
            } else if name.to_uppercase().contains("AMD") || name.to_uppercase().contains("INTEL") {
                // Assume the first non-NVIDIA is iGPU
                if igpu_label == "iGPU" {
                    igpu_label = name.replace(" Radeon Graphics", "");
                }
            }
        }
    }

    let (cpu_row, cpu_temp_l, cpu_power_l, cpu_dot, cpu_util_l) = make_row(&cpu_label);
    let (igpu_row, igpu_temp_l, igpu_power_l, igpu_dot, igpu_util_l) = make_row(&igpu_label);
    let (dgpu_row, dgpu_temp_l, dgpu_power_l, dgpu_dot, dgpu_util_l) = make_row(&dgpu_label);

    // Battery + Fan row (status + watts on left, fan on right)
    let bottom_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    bottom_row.set_margin_top(1);
    bottom_row.set_margin_bottom(1);

    let bat_status_l = gtk::Label::new(None);
    bat_status_l.add_css_class("caption");
    bat_status_l.set_xalign(0.0);
    bat_status_l.set_opacity(0.6);
    let bat_pct_l = gtk::Label::new(None);
    bat_pct_l.add_css_class("caption");
    bat_pct_l.add_css_class("numeric");
    let bat_watts_l = gtk::Label::new(None);
    bat_watts_l.add_css_class("caption");
    bat_watts_l.add_css_class("numeric");
    bat_watts_l.set_opacity(0.7);

    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);

    let fan_l = gtk::Label::new(None);
    fan_l.add_css_class("caption");
    fan_l.add_css_class("numeric");
    fan_l.set_opacity(0.6);

    bottom_row.append(&bat_status_l);
    bottom_row.append(&bat_pct_l);
    bottom_row.append(&bat_watts_l);
    bottom_row.append(&spacer);
    bottom_row.append(&fan_l);

    main_box.append(&cpu_row);
    main_box.append(&igpu_row);
    main_box.append(&dgpu_row);
    main_box.append(&bottom_row);

    glib::timeout_add_local(Duration::from_secs(2), move || {
        let nvidia = gpu_monitor::read_nvidia_telemetry();
        let cpu_temp = get_cpu_temperature();
        let igpu_temp = get_igpu_temperature();
        let dgpu_temp = nvidia.temperature;
        let on_ac = check_if_running_on_ac_power();
        let ac = on_ac.unwrap_or(true);
        let fan = get_fan_speed(ac);
        let battery_pct = get_battery_percentage();
        let battery_status = get_battery_status();
        let battery_power = get_battery_power();
        let sys_power = get_system_power();
        let cpu_util = get_cpu_utilization();
        let igpu_pwr = get_igpu_power();
        let igpu_util = get_igpu_utilization();
        let dgpu_pwr = nvidia.power;
        let dgpu_util = nvidia.utilization;

        // CPU
        match cpu_temp {
            Some(t) => {
                cpu_temp_l.set_text(&format!("{:.0}\u{00B0}C", t));
                cpu_row.set_visible(true);
            }
            None => cpu_row.set_visible(false),
        }
        match sys_power {
            Some(w) => {
                cpu_power_l.set_text(&format!("{:.1} W", w));
                cpu_power_l.set_visible(true);
            }
            None => cpu_power_l.set_visible(false),
        }
        match cpu_util {
            Some(u) => {
                cpu_util_l.set_text(&format!("{}%", u));
                cpu_util_l.set_visible(true);
                cpu_dot.set_visible(sys_power.is_some());
            }
            None => {
                cpu_util_l.set_visible(false);
                cpu_dot.set_visible(false);
            }
        }

        // iGPU
        let igpu_has = igpu_temp.is_some() || igpu_pwr.is_some();
        igpu_row.set_visible(igpu_has);
        if igpu_has {
            match igpu_temp {
                Some(t) => {
                    igpu_temp_l.set_text(&format!("{:.0}\u{00B0}C", t));
                    igpu_temp_l.set_visible(true);
                }
                None => igpu_temp_l.set_visible(false),
            }
            match igpu_pwr {
                Some(w) => {
                    igpu_power_l.set_text(&format!("{:.1} W", w));
                    igpu_power_l.set_visible(true);
                }
                None => igpu_power_l.set_visible(false),
            }
            match igpu_util {
                Some(u) => {
                    igpu_util_l.set_text(&format!("{}%", u));
                    igpu_util_l.set_visible(true);
                    igpu_dot.set_visible(igpu_pwr.is_some());
                }
                None => {
                    igpu_util_l.set_visible(false);
                    igpu_dot.set_visible(false);
                }
            }
        }

        // dGPU
        let dgpu_has = dgpu_temp.is_some() || dgpu_pwr.is_some() || dgpu_util.is_some();
        dgpu_row.set_visible(dgpu_has);
        if dgpu_has {
            match dgpu_temp {
                Some(t) => {
                    dgpu_temp_l.set_text(&format!("{:.0}\u{00B0}C", t));
                    dgpu_temp_l.set_visible(true);
                }
                None => dgpu_temp_l.set_visible(false),
            }
        }
        match dgpu_pwr {
            Some(w) => {
                dgpu_power_l.set_text(&format!("{:.1} W", w));
                dgpu_power_l.set_visible(true);
            }
            None => dgpu_power_l.set_visible(false),
        }
        match dgpu_util {
            Some(u) => {
                dgpu_util_l.set_text(&format!("{}%", u));
                dgpu_util_l.set_visible(true);
                dgpu_dot.set_visible(dgpu_pwr.is_some());
            }
            None => {
                dgpu_util_l.set_visible(false);
                dgpu_dot.set_visible(false);
            }
        }

        // Battery + Fan bottom row
        match battery_pct {
            Some(pct) => {
                let status_text = match battery_status.as_deref() {
                    Some("Charging") => "Charging",
                    Some("Not charging") => "Full (Limit)",
                    Some("Full") => "Full",
                    Some("Discharging") => "Battery",
                    _ => "Battery",
                };
                bat_status_l.set_text(status_text);
                bat_pct_l.set_text(&format!("{}%", pct));
                bat_status_l.set_visible(true);
                bat_pct_l.set_visible(true);
                match (battery_status.as_deref(), battery_power) {
                    (Some("Charging"), Some(w)) => {
                        bat_watts_l.set_text(&format!("+{:.1}W", w));
                        bat_watts_l.set_visible(true);
                    }
                    (Some("Discharging"), Some(w)) => {
                        bat_watts_l.set_text(&format!("\u{2212}{:.1}W", w));
                        bat_watts_l.set_visible(true);
                    }
                    _ => bat_watts_l.set_visible(false),
                }
            }
            None => {
                bat_status_l.set_visible(false);
                bat_pct_l.set_visible(false);
                bat_watts_l.set_visible(false);
            }
        }

        match fan {
            Some(0) => {
                fan_l.set_text("Fan: Auto");
                fan_l.set_visible(true);
            }
            Some(rpm) => {
                fan_l.set_text(&format!("Fan: {} RPM", rpm));
                fan_l.set_visible(true);
            }
            None => fan_l.set_visible(false),
        }

        // Write snapshot to shared state for tray tooltip
        if let Ok(mut state) = shared_state.lock() {
            state.cpu_temp = cpu_temp;
            state.igpu_temp = igpu_temp;
            state.dgpu_temp = dgpu_temp;
            state.fan_speed = fan;
            state.on_ac = on_ac;
            state.battery_pct = battery_pct;
            state.battery_status = battery_status.clone();
            state.battery_power = battery_power;
            state.system_power = sys_power;
            state.cpu_util = cpu_util;
            state.igpu_power = igpu_pwr;
            state.igpu_util = igpu_util;
            state.dgpu_power = dgpu_pwr;
            state.dgpu_util = dgpu_util;
        }

        glib::ControlFlow::Continue
    });

    main_box
}

fn main() {
    // Checked before anything else (GTK, D-Bus, panic hook) touches process-wide state - a
    // duplicate launch should exit as cheaply and early as possible.
    let _instance_lock = match acquire_single_instance_lock() {
        InstanceLock::Acquired(file) => Some(file),
        InstanceLock::AlreadyRunning(pid) => {
            match pid {
                Some(pid) => {
                    eprintln!(
                        "razer-settings is already running (pid {pid}) - raising its window."
                    );
                    // SAFETY: kill() with a plain signal number and a PID we just parsed from
                    // our own lock file has no memory-safety implications; a delivery failure
                    // (e.g. the process exited between our read and this call) is handled below
                    // exactly like "no PID available" - this process still just exits.
                    unsafe {
                        libc::kill(pid, FOCUS_SIGNAL);
                    }
                }
                None => {
                    eprintln!(
                        "razer-settings is already running - not starting a second instance."
                    );
                }
            }
            std::process::exit(0);
        }
        InstanceLock::Unavailable => None,
    };

    setup_panic_hook();

    // Shared sensor state for tray tooltip
    let shared_state = tray::new_shared_state();

    let app = adw::Application::builder()
        .application_id("io.github.stizzy98.openrazer")
        .flags(gtk::gio::ApplicationFlags::empty())
        .build();

    // Keep the app alive even when the window is hidden (close-to-tray)
    let _hold_guard = app.hold();

    // A duplicate launch signals FOCUS_SIGNAL (see acquire_single_instance_lock) instead of
    // opening a second window. Signal handlers can't safely touch GTK directly, so the actual
    // handler just flips this flag from a background thread; a main-loop timeout below (added
    // once the window exists, in connect_activate) polls it and does the real `window.present()`
    // on the correct thread.
    let focus_requested = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let focus_requested = Arc::clone(&focus_requested);
        std::thread::spawn(move || {
            let Ok(mut signals) = signal_hook::iterator::Signals::new([FOCUS_SIGNAL]) else {
                return;
            };
            for _ in signals.forever() {
                focus_requested.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        });
    }

    // Spawn tray only on primary instance (inside connect_startup)
    let tray_state = Arc::clone(&shared_state);
    app.connect_startup(move |_| {
        adw::init().ok();

        let style_manager = adw::StyleManager::default();
        style_manager.set_color_scheme(adw::ColorScheme::ForceDark);

        let provider = gtk::CssProvider::new();
        provider.load_from_string(&omarchy_theme::build_css(&omarchy_theme::load()));
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().expect("Could not connect to a display"),
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        // Spawn KDE system tray icon (only on primary instance)
        let tray = tray::RazerTray::new(Arc::clone(&tray_state));
        {
            use ksni::blocking::TrayMethods;
            match tray.spawn() {
                Ok(_handle) => {} // tray runs in background thread
                Err(e) => eprintln!("Tray error (non-fatal): {}", e),
            }
        }
    });

    let shared_state_for_activate = Arc::clone(&shared_state);
    let focus_requested_for_activate = Arc::clone(&focus_requested);
    app.connect_activate(move |app| {
        // If a window already exists (even if hidden), show it
        let windows = app.windows();
        if !windows.is_empty() {
            let win = &windows[0];
            win.set_visible(true);
            win.present();
            return;
        }

        let device_file = std::fs::read_to_string(service::device_file_path()).unwrap_or("[]".into());
        let devices: Vec<SupportedDevice> = match serde_json::from_str(&device_file) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Failed to parse device file: {}", e);
                show_error_dialog(app, &format!(
                    "Failed to parse device file ({}).\n\nPlease ensure razercontrol is installed correctly.",
                    e
                ));
                return;
            }
        };

        let device_name = match get_device_name() {
            Some(name) => name,
            None => {
                eprintln!("Failed to get device name from daemon");
                show_error_dialog(app,
                    "Failed to get device name.\n\n\
                    The daemon may not be running or failed to respond.\n\
                    Try: systemctl --user restart razercontrol"
                );
                return;
            }
        };

        let device = match devices.iter().find(|d| d.name == device_name) {
            Some(d) => d.clone(),
            None => {
                eprintln!("Device '{}' not found in laptops.json", device_name);
                show_error_dialog(app, &format!(
                    "Device '{}' not found in supported devices list.\n\n\
                    Your device may not be supported yet, or the device database is outdated.",
                    device_name
                ));
                return;
            }
        };

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Razer Control")
            .default_width(850)
            .default_height(700)
            .build();

        // Close-to-tray: hide window instead of quitting
        window.connect_close_request(|win| {
            win.set_visible(false);
            glib::Propagation::Stop
        });

        let content_box = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let header_bar = adw::HeaderBar::new();
        header_bar.set_show_end_title_buttons(true);

        // Small, functional nod to the live theme-reading above - shows which Omarchy theme this
        // session's colors came from, rather than silent decoration.
        if let Some(theme_name) = omarchy_theme::load().theme_name {
            let theme_label = gtk::Label::new(Some(&theme_name));
            theme_label.add_css_class("dim-label");
            theme_label.add_css_class("caption");
            header_bar.pack_end(&theme_label);
        }

        let view_switcher = adw::ViewSwitcher::new();
        view_switcher.set_policy(adw::ViewSwitcherPolicy::Wide);

        let view_stack = adw::ViewStack::new();
        view_switcher.set_stack(Some(&view_stack));
        header_bar.set_title_widget(Some(&view_switcher));

        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(900);
        clamp.set_tightening_threshold(600);

        let scrolled_window = gtk::ScrolledWindow::new();
        scrolled_window.set_vexpand(true);
        scrolled_window.set_hscrollbar_policy(gtk::PolicyType::Never);
        scrolled_window.set_child(Some(&clamp));
        clamp.set_child(Some(&view_stack));

        content_box.append(&header_bar);
        content_box.append(&scrolled_window);

        // Lighting page
        let lighting_page = make_lighting_page(device.clone());
        let page = view_stack.add_titled(&lighting_page.page, Some("Lighting"), "Lighting");
        page.set_icon_name(Some("display-brightness-symbolic"));

        // Performance page
        let perf_page = make_performance_page(device.clone());
        let page = view_stack.add_titled(&perf_page.page, Some("Performance"), "Performance");
        page.set_icon_name(Some("power-profile-balanced-symbolic"));

        // Battery page
        let battery_page = make_battery_page();
        let page = view_stack.add_titled(&battery_page.page, Some("Battery"), "Battery");
        page.set_icon_name(Some("battery-symbolic"));

        // (GPU sections are now part of the Performance page)

        // About page
        let about_page = make_about_page(device.clone());
        let page = view_stack.add_titled(&about_page.page, Some("About"), "About");
        page.set_icon_name(Some("help-about-symbolic"));

        let monitor = create_system_monitor(Arc::clone(&shared_state_for_activate));
        content_box.append(&monitor);

        let toast_overlay = adw::ToastOverlay::new();
        toast_overlay.set_child(Some(&content_box));

        window.set_content(Some(&toast_overlay));
        // A restart after an update keeps the window hidden if it was closed to the tray.
        if std::env::var_os(RESTART_HIDDEN_ENV).is_none() {
            window.present();
        }

        // Restart into the new version when this binary is replaced by a reinstall or update,
        // rather than leaving a stale copy running in the tray against a newer daemon.
        let update_window = window.clone();
        let mut update_watch = service::UpdateWatch::default();
        glib::timeout_add_local(Duration::from_secs(2), move || {
            if let Some(exe) = update_watch.poll() {
                restart_into(&exe, update_window.is_visible());
                // Only reached if exec failed - keep running this version rather than retrying.
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });

        // Poll for a duplicate-launch focus request (see FOCUS_SIGNAL) and raise this window
        // when one arrives. This closure only ever runs on the main loop, so touching `window`
        // here is safe even though the flag itself is flipped from a background signal thread.
        let focus_window = window.clone();
        let focus_requested = Arc::clone(&focus_requested_for_activate);
        glib::timeout_add_local(Duration::from_millis(200), move || {
            if focus_requested.swap(false, std::sync::atomic::Ordering::SeqCst) {
                focus_window.set_visible(true);
                focus_window.present();
            }
            glib::ControlFlow::Continue
        });

    });

    app.run();
}

// ---------------------------------------------------------------------------
// Performance page
// ---------------------------------------------------------------------------

fn make_performance_page(device: SupportedDevice) -> SettingsPage {
    let settings_page = SettingsPage::new();

    // AC / Battery toggle
    let (toggle_box, is_ac) = make_profile_toggle();
    let refreshing = Rc::new(Cell::new(false));

    // We'll add the toggle to the first group's header
    let toggle_section = settings_page.add_section(None);
    toggle_section.add_row(&toggle_box);

    // --- Power Profile section ---
    let power_section = settings_page.add_section(Some("Power Profile"));

    let initial_ac = is_ac.get();
    let power = get_power(initial_ac);

    let power_combo = make_combo_row(
        "Profile",
        profile_description(power.map_or(0, |p| p.0 as u32)),
        &["Balanced", "Gaming", "Creator", "Silent", "Custom"],
        power.map_or(0, |p| p.0 as u32),
    );
    power_section.add_row(&power_combo);

    let cpu_options: &[&str] = if device.can_boost() {
        &["Low", "Medium", "High", "Boost"]
    } else {
        &["Low", "Medium", "High"]
    };
    let cpu_combo = make_combo_row(
        "CPU Performance",
        "Processor performance level",
        cpu_options,
        power.map_or(0, |p| p.1 as u32),
    );
    power_section.add_row(&cpu_combo);

    let gpu_combo = make_combo_row(
        "GPU Performance",
        "Graphics performance level",
        &["Low", "Medium", "High"],
        power.map_or(0, |p| p.2 as u32),
    );
    power_section.add_row(&gpu_combo);

    let show_boost = power.is_some_and(|p| p.0 == 4);
    cpu_combo.set_visible(show_boost);
    gpu_combo.set_visible(show_boost);

    // --- Cooling section ---
    let fan_section = settings_page.add_section(Some("Cooling"));

    let fan_speed = get_fan_speed(initial_ac).unwrap_or(0);
    let min_fan_speed = *device.fan.get(0).unwrap_or(&0) as f64;
    let max_fan_speed = *device.fan.get(1).unwrap_or(&5000) as f64;
    let auto = fan_speed == 0;

    let fan_switch = make_switch_row(
        "Automatic Fan Control",
        "Let the system manage fan speed",
        auto,
    );
    fan_section.add_row(&fan_switch);

    let fan_slider = SliderRow::new(
        "Fan Speed (RPM)",
        "Manual cooling performance",
        min_fan_speed,
        max_fan_speed,
        100.0,
        if auto {
            min_fan_speed
        } else {
            fan_speed as f64
        },
    );
    fan_slider.add_mark(min_fan_speed, Some("Min"));
    fan_slider.add_mark(max_fan_speed, Some("Max"));
    fan_slider.scale.set_sensitive(!auto);
    fan_section.add_row(&fan_slider.container);

    // --- Callbacks ---

    // Refresh helper: re-query daemon and update all widgets on this page
    let refresh = {
        let is_ac = is_ac.clone();
        let refreshing = refreshing.clone();
        let power_combo = power_combo.clone();
        let cpu_combo = cpu_combo.clone();
        let gpu_combo = gpu_combo.clone();
        let fan_switch = fan_switch.clone();
        let fan_scale = fan_slider.scale.clone();
        let min_fan = min_fan_speed;
        move || {
            refreshing.set(true);
            let ac = is_ac.get();
            if let Some(pwr) = get_power(ac) {
                power_combo.set_selected(pwr.0 as u32);
                power_combo.set_subtitle(profile_description(pwr.0 as u32));
                cpu_combo.set_selected(pwr.1 as u32);
                gpu_combo.set_selected(pwr.2 as u32);
                let show = pwr.0 == 4;
                cpu_combo.set_visible(show);
                gpu_combo.set_visible(show);
            }
            let fs = get_fan_speed(ac).unwrap_or(0);
            let auto = fs == 0;
            fan_switch.set_active(auto);
            fan_scale.set_sensitive(!auto);
            if !auto {
                fan_scale.set_value(fs as f64);
            } else {
                fan_scale.set_value(min_fan);
            }
            refreshing.set(false);
        }
    };

    // Toggle callback — hook the actual toggle buttons to refresh page state
    {
        let first_child = toggle_box.first_child();
        if let Some(ac_btn) = first_child
            && let Ok(tb) = ac_btn.downcast::<gtk::ToggleButton>()
        {
            let refresh = refresh.clone();
            tb.connect_toggled(move |btn| {
                if btn.is_active() {
                    refresh();
                }
            });
        }
        let last_child = toggle_box.last_child();
        if let Some(bat_btn) = last_child
            && let Ok(tb) = bat_btn.downcast::<gtk::ToggleButton>()
        {
            let refresh = refresh.clone();
            tb.connect_toggled(move |btn| {
                if btn.is_active() {
                    refresh();
                }
            });
        }
    }

    // Power profile change
    {
        let is_ac = is_ac.clone();
        let refreshing = refreshing.clone();
        let cpu_combo = cpu_combo.clone();
        let gpu_combo = gpu_combo.clone();
        power_combo.connect_selected_notify(glib::clone!(
            #[weak]
            cpu_combo,
            #[weak]
            gpu_combo,
            move |pp| {
                if refreshing.get() {
                    return;
                }
                let ac = is_ac.get();
                let profile = pp.selected() as u8;
                let cpu = cpu_combo.selected() as u8;
                let gpu = gpu_combo.selected() as u8;
                set_power(ac, (profile, cpu, gpu));
                pp.set_subtitle(profile_description(profile as u32));
                let show = profile == 4;
                cpu_combo.set_visible(show);
                gpu_combo.set_visible(show);
            }
        ));
    }

    {
        let is_ac = is_ac.clone();
        let refreshing = refreshing.clone();
        let power_combo = power_combo.clone();
        let gpu_combo = gpu_combo.clone();
        cpu_combo.connect_selected_notify(glib::clone!(
            #[weak]
            power_combo,
            #[weak]
            gpu_combo,
            move |cb| {
                if refreshing.get() {
                    return;
                }
                let ac = is_ac.get();
                let profile = power_combo.selected() as u8;
                let cpu = cb.selected() as u8;
                let gpu = gpu_combo.selected() as u8;
                set_power(ac, (profile, cpu, gpu));
            }
        ));
    }

    {
        let is_ac = is_ac.clone();
        let refreshing = refreshing.clone();
        let power_combo = power_combo.clone();
        let cpu_combo = cpu_combo.clone();
        gpu_combo.connect_selected_notify(glib::clone!(
            #[weak]
            power_combo,
            #[weak]
            cpu_combo,
            move |gb| {
                if refreshing.get() {
                    return;
                }
                let ac = is_ac.get();
                let profile = power_combo.selected() as u8;
                let cpu = cpu_combo.selected() as u8;
                let gpu = gb.selected() as u8;
                set_power(ac, (profile, cpu, gpu));
            }
        ));
    }

    // Fan control callbacks
    {
        let is_ac = is_ac.clone();
        let refreshing = refreshing.clone();
        let fan_switch_ref = fan_switch.clone();
        fan_slider.scale.connect_value_changed(move |sc| {
            if refreshing.get() {
                return;
            }
            let ac = is_ac.get();
            let value = sc.value();
            set_fan_speed(ac, value as i32);
            // Flipping the switch here is a side effect of this drag, not a real user toggle -
            // guard it so the switch's own handler doesn't re-fire and stomp the value we just
            // sent back down to the minimum (it used to, since it has no way to tell the two
            // apart otherwise).
            refreshing.set(true);
            fan_switch_ref.set_active(false);
            refreshing.set(false);
        });
    }

    {
        let is_ac = is_ac.clone();
        let refreshing_ref = refreshing.clone();
        let scale_ref = fan_slider.scale.clone();
        fan_switch.connect_active_notify(glib::clone!(
            #[weak]
            scale_ref,
            move |sw| {
                if refreshing_ref.get() {
                    return;
                }
                let ac = is_ac.get();
                let state = sw.is_active();
                if state {
                    set_fan_speed(ac, 0);
                } else {
                    set_fan_speed(ac, min_fan_speed as i32);
                    scale_ref.set_value(min_fan_speed);
                }
                scale_ref.set_sensitive(!state);
            }
        ));
    }

    // -----------------------------------------------------------------------
    // GPU sections (merged from former GPU page)
    // -----------------------------------------------------------------------
    let gpu_refreshing = Rc::new(Cell::new(false));
    let gpu_cooldown = Rc::new(Cell::new(false));
    let gpu_status = get_gpu_status();

    // --- Detected GPUs ---
    let gpu_section = settings_page.add_section(Some("Detected GPUs"));
    let gpu_rows: Vec<adw::ActionRow> = if let Some((ref gpus, _, _, _)) = gpu_status {
        gpus.iter()
            .map(|gpu| {
                let row = adw::ActionRow::new();
                row.set_title(&gpu.name);
                let type_label = if gpu.gpu_type == "dgpu" {
                    "Discrete"
                } else {
                    "Integrated"
                };
                row.set_subtitle(&format!(
                    "{} \u{00B7} {} \u{00B7} {} \u{00B7} {}",
                    type_label, gpu.pci_slot, gpu.driver, gpu.runtime_status
                ));
                gpu_section.add_row(&row);
                row
            })
            .collect()
    } else {
        let row = adw::ActionRow::new();
        row.set_title("No GPUs detected");
        row.set_subtitle("Could not query GPU information from daemon");
        gpu_section.add_row(&row);
        vec![row]
    };

    // --- dGPU Runtime Power ---
    let has_dgpu = gpu_status
        .as_ref()
        .is_some_and(|(gpus, _, _, _)| gpus.iter().any(|g| g.gpu_type == "dgpu"));
    let dgpu_rpm_active = gpu_status.as_ref().is_some_and(|(_, rpm, _, _)| *rpm);

    let rpm_section = settings_page.add_section(Some("dGPU Runtime Power"));
    let rpm_switch = make_switch_row(
        "Suspend dGPU",
        "Allow the discrete GPU to power down when idle (instant, no reboot)",
        dgpu_rpm_active,
    );
    rpm_section.add_row(&rpm_switch);

    if !has_dgpu {
        rpm_switch.set_sensitive(false);
        rpm_switch.set_subtitle("No discrete GPU detected");
    }

    // dGPU switch callback — with cooldown to prevent live-sync from reverting
    {
        let gpu_refreshing = gpu_refreshing.clone();
        let gpu_cooldown = gpu_cooldown.clone();
        rpm_switch.connect_active_notify(move |sw| {
            if gpu_refreshing.get() {
                return;
            }
            set_dgpu_runtime_pm(sw.is_active());
            // Set cooldown so the live-sync skips the next few polls
            gpu_cooldown.set(true);
            let cd = gpu_cooldown.clone();
            glib::timeout_add_local_once(Duration::from_secs(4), move || {
                cd.set(false);
            });
        });
    }

    // --- envycontrol GPU Mode ---
    let ec_available = gpu_status.as_ref().is_some_and(|(_, _, _, avail)| *avail);
    let ec_mode = gpu_status
        .as_ref()
        .map_or("unknown".to_string(), |(_, _, mode, _)| mode.clone());

    let ec_section = settings_page.add_section(Some("GPU Mode (envycontrol)"));

    if ec_available {
        let mode_idx = match ec_mode.as_str() {
            "hybrid" => 0u32,
            "integrated" => 1,
            "nvidia" => 2,
            _ => 0,
        };

        let mode_combo = make_combo_row(
            "GPU Mode",
            gpu_mode_description(mode_idx),
            &["Hybrid", "Integrated", "NVIDIA Only"],
            mode_idx,
        );
        ec_section.add_row(&mode_combo);

        let info_label = gtk::Label::new(Some("Changing GPU mode requires logout to take effect."));
        info_label.set_wrap(true);
        info_label.add_css_class("dim-label");
        info_label.add_css_class("caption");
        info_label.set_margin_top(4);
        info_label.set_margin_bottom(8);
        info_label.set_margin_start(12);
        info_label.set_margin_end(12);
        ec_section.add_row(&info_label);

        // Auto-apply callback on selection change
        {
            let mode_combo = mode_combo.clone();
            let gpu_refreshing = gpu_refreshing.clone();

            // Capture a weak reference or solve the root access differently.
            // We can't capture the widget itself and use it easily if we also clone it?
            // construct logic inside.

            mode_combo.clone().connect_selected_notify(move |c| {
                // Update subtitle
                c.set_subtitle(gpu_mode_description(c.selected()));

                if gpu_refreshing.get() {
                    return;
                }

                let mode_str = match c.selected() {
                    0 => "hybrid",
                    1 => "integrated",
                    2 => "nvidia",
                    _ => "hybrid",
                };
                let mode_owned = mode_str.to_string();

                // Attempt to find toast overlay
                let overlay_ref: Option<adw::ToastOverlay> = c
                    .root()
                    .and_then(|r| r.downcast::<adw::ApplicationWindow>().ok())
                    .and_then(|w| w.content())
                    .and_then(|c| c.downcast::<adw::ToastOverlay>().ok());

                // Perform the action
                let (msg, timeout) = match set_gpu_mode(&mode_owned) {
                    Some((true, _)) => (
                        format!("GPU mode set to '{}' \u{2014} log out to apply", mode_owned),
                        3,
                    ),
                    Some((false, msg)) => (format!("Failed: {}", msg), 4),
                    None => ("Failed to communicate with daemon".to_string(), 4),
                };

                // Show toast
                if let Some(ref o) = overlay_ref {
                    let toast = adw::Toast::new(&msg);
                    toast.set_timeout(timeout);
                    o.add_toast(toast);
                } else {
                    // Fallback to stderr if UI not ready (unlikely)
                    eprintln!("{}", msg);
                }
            });
        }
    } else {
        let info_label = gtk::Label::new(Some(
            "envycontrol is not installed. Install it for persistent GPU mode switching.",
        ));
        info_label.set_wrap(true);
        info_label.add_css_class("dim-label");
        info_label.set_margin_top(12);
        info_label.set_margin_bottom(12);
        info_label.set_margin_start(12);
        info_label.set_margin_end(12);
        ec_section.add_row(&info_label);
    }

    // -----------------------------------------------------------------------
    // Combined live-sync: poll performance + GPU every 2s
    // -----------------------------------------------------------------------
    {
        let refresh = refresh.clone();
        let gpu_refreshing = gpu_refreshing.clone();
        let gpu_cooldown = gpu_cooldown.clone();
        let rpm_switch = rpm_switch.clone();
        let gpu_rows = gpu_rows.clone();
        glib::timeout_add_local(Duration::from_secs(2), move || {
            // Performance refresh
            refresh();

            // GPU refresh (skip if user just toggled the switch)
            if !gpu_cooldown.get()
                && let Some((gpus, dgpu_rpm, _, _)) = get_gpu_status()
            {
                gpu_refreshing.set(true);
                rpm_switch.set_active(dgpu_rpm);
                for (i, row) in gpu_rows.iter().enumerate() {
                    if let Some(gpu) = gpus.get(i) {
                        let type_label = if gpu.gpu_type == "dgpu" {
                            "Discrete"
                        } else {
                            "Integrated"
                        };
                        row.set_subtitle(&format!(
                            "{} \u{00B7} {} \u{00B7} {} \u{00B7} {}",
                            type_label, gpu.pci_slot, gpu.driver, gpu.runtime_status
                        ));
                    }
                }
                gpu_refreshing.set(false);
            }

            glib::ControlFlow::Continue
        });
    }

    settings_page
}

// ---------------------------------------------------------------------------
// Lighting page
// ---------------------------------------------------------------------------

fn make_lighting_page(device: SupportedDevice) -> SettingsPage {
    let settings_page = SettingsPage::new();

    // AC / Battery toggle (affects brightness + logo only)
    let (toggle_box, is_ac) = make_profile_toggle();
    let refreshing = Rc::new(Cell::new(false));

    let toggle_section = settings_page.add_section(None);
    toggle_section.add_row(&toggle_box);

    // --- Keyboard Brightness ---
    let brightness_section = settings_page.add_section(Some("Keyboard Brightness"));

    let initial_ac = is_ac.get();
    let brightness = get_brightness(initial_ac).unwrap_or(100);

    let brightness_slider = SliderRow::new(
        "Brightness Level",
        "Adjust keyboard backlight intensity",
        0.0,
        100.0,
        1.0,
        brightness as f64,
    );
    brightness_slider.add_mark(0.0, Some("Off"));
    brightness_slider.add_mark(50.0, Some("50%"));
    brightness_slider.add_mark(100.0, Some("100%"));
    brightness_section.add_row(&brightness_slider.container);

    // --- Logo (conditional) ---
    let logo_combo: Option<adw::ComboRow> = if device.has_logo() {
        let logo_section = settings_page.add_section(Some("Logo"));
        let logo = get_logo(initial_ac).unwrap_or(1);
        let combo = make_combo_row(
            "Logo Mode",
            "Control Razer logo lighting",
            &["Off", "On", "Breathing"],
            logo as u32,
        );
        logo_section.add_row(&combo);
        Some(combo)
    } else {
        None
    };

    // --- Keyboard Effects (GLOBAL — not affected by AC/Battery toggle) ---
    // Every effect here is a *native* hardware effect sent via SetStandardEffect - the daemon
    // already implements and correctly parameterises all 7 (proven daily via `razer-cli
    // standard-effect ...`); the old SetEffect-based 4-item combo this replaces silently sent
    // mismatched parameter byte-layouts on hardware without per_key_rgb (e.g. Breathing sent
    // [r,g,b,duration] into a report that expects [kind,r1,g1,b1,r2,g2,b2]), so it never needs
    // to be called from the GUI again.
    let effects_section = settings_page.add_section(Some("Keyboard Effects"));

    // Index = value sent to the daemon's SetStandardEffect name match, see EFFECT_NAMES below.
    const EFFECT_NAMES: [&str; 7] = [
        "off",
        "static",
        "wave",
        "breathing",
        "reactive",
        "spectrum",
        "starlight",
    ];
    const OFF: u32 = 0;
    const STATIC: u32 = 1;
    const WAVE: u32 = 2;
    const BREATHING: u32 = 3;
    const REACTIVE: u32 = 4;
    const SPECTRUM: u32 = 5;
    const STARLIGHT: u32 = 6;
    // Wheel is NOT part of EFFECT_NAMES/SetStandardEffect - the hardware's native Wheel effect
    // never produced any visible output on this laptop, so it's a real software animation (a
    // rotating rainbow, see kbd::effects::Wheel) sent via its own SetWheelEffect command instead.
    const WHEEL: u32 = 7;
    // Audio Meter is also a software effect, like Wheel - a real single-level VU meter (the whole
    // keyboard rises/falls together with overall volume, not a per-band spectrum analyzer - see
    // kbd::effects::SoundBar) sent via its own SetSoundBarEffect command.
    const SOUNDBAR: u32 = 8;
    // Stars is also a software effect - per-key random colors, re-rolled on a fixed interval
    // (see kbd::effects::Stars) - sent via its own SetStarsEffect command.
    const STARS: u32 = 9;
    // Ripple is also a software effect - rings spreading out from each key press (see
    // kbd::effects::Ripple) - sent via its own SetRippleEffect command.
    const RIPPLE: u32 = 10;
    // Real firmware "type" byte for Breathing/Starlight, confirmed against OpenRazer's
    // razerchromacommon.c (razer_chroma_standard_matrix_effect_{breathing,starlight}_*): the
    // wire value is 1/2/3, NOT 0-indexed - sending 0 for "Single" hit an unrecognized type and
    // produced undefined behavior (confirmed live: selecting Single actually ran Random, because
    // this project's own combo previously used 0/1/2). mode_combo's `selected()` is a normal
    // 0-based GTK list index (0=Single/1=Dual/2=Random) - always convert with +1 when sending,
    // -1 when restoring; never compare a raw `selected()` value against these constants directly.
    const MODE_SINGLE: u32 = 1;
    const MODE_DUAL: u32 = 2;
    const MODE_RANDOM: u32 = 3;

    /// `GetStandardEffect`/`config.standard_effect` store the *raw hardware* RazerLaptop effect
    /// byte (OFF=0x00, WAVE=0x01, REACTIVE=0x02, BREATHING=0x03, SPECTRUM=0x04, STATIC=0x06,
    /// STARLIGHT=0x19 - see daemon/device.rs), which is a completely different numbering from
    /// this combo's display order - mixing them up silently selects the wrong row on restore and
    /// misparses its params (confirmed live: a real Reactive effect's raw id 0x02 collided with
    /// this combo's own index 2, "Wave"). Always go through this mapping, never compare the raw
    /// byte against the combo constants directly.
    fn combo_index_for_hw_id(hw_id: u8) -> Option<u32> {
        match hw_id {
            0x00 => Some(OFF),
            0x06 => Some(STATIC),
            0x01 => Some(WAVE),
            0x03 => Some(BREATHING),
            0x02 => Some(REACTIVE),
            0x04 => Some(SPECTRUM),
            0x19 => Some(STARLIGHT),
            _ => None,
        }
    }

    /// Which rows should be visible for a given (effect, color-mode) combination - shared by the
    /// initial layout, both combos' change handlers, and effect restore-on-open, so the
    /// visibility rules only ever live in one place.
    /// `ripple_static` is whether Ripple's own color-mode combo is on Static (it only needs a
    /// color then).
    fn visible_for(
        effect: u32,
        mode: u32,
        ripple_static: bool,
    ) -> (bool, bool, bool, bool, bool, bool, bool, bool, bool) {
        // (mode_combo, color1, color2, direction_combo, speed_slider, wheel_speed_slider, sound_bar_mode_combo, stars_speed_slider, ripple_rows)
        match effect {
            OFF | SPECTRUM => (
                false, false, false, false, false, false, false, false, false,
            ),
            STATIC => (false, true, false, false, false, false, false, false, false),
            WAVE => (false, false, false, true, false, false, false, false, false),
            REACTIVE => (false, true, false, false, true, false, false, false, false),
            BREATHING => (
                true,
                mode != MODE_RANDOM,
                mode == MODE_DUAL,
                false,
                false,
                false,
                false,
                false,
                false,
            ),
            STARLIGHT => (
                true,
                mode != MODE_RANDOM,
                mode == MODE_DUAL,
                false,
                true,
                false,
                false,
                false,
                false,
            ),
            WHEEL => (false, false, false, true, false, true, false, false, false),
            // color1 stays visible regardless of Rainbow/Static/VU sub-mode - harmless when
            // unused (Rainbow/VU ignore it), simpler than a second layer of conditional visibility.
            SOUNDBAR => (false, true, false, false, false, false, true, false, false),
            STARS => (false, false, false, false, false, false, false, true, false),
            RIPPLE => (
                false,
                ripple_static,
                false,
                false,
                false,
                false,
                false,
                false,
                true,
            ),
            _ => (
                false, false, false, false, false, false, false, false, false,
            ),
        }
    }

    let effect_combo = make_combo_row(
        "Effect Type",
        "Choose keyboard lighting effect",
        &[
            "Off",
            "Static",
            "Wave",
            "Breathing",
            "Reactive",
            "Spectrum",
            "Starlight",
            "Wheel",
            "Audio Meter",
            "Stars",
            "Ripple",
        ],
        STATIC,
    );
    effects_section.add_row(&effect_combo);

    let mode_combo = make_combo_row(
        "Color Mode",
        "Single, dual-color, or a random color each cycle",
        &["Single", "Dual", "Random"],
        MODE_SINGLE,
    );
    effects_section.add_row(&mode_combo);

    // GTK4's ColorDialog (the portal-based dialog GtkColorDialogButton opens) has no API at all
    // to customize its own preset palette - confirmed directly against the gtk4-rs bindings, not
    // guessed. This is a separate, plain-button strip that sets a target ColorDialogButton's
    // color directly (no dialog involved), so there's always a wider, one-click preset set
    // available regardless of that platform limitation.
    fn make_quick_colors_row(target: &gtk::ColorDialogButton, id_prefix: &str) -> gtk::Box {
        const PRESETS: [(&str, u8, u8, u8); 12] = [
            ("Red", 255, 0, 0),
            ("Orange", 255, 128, 0),
            ("Yellow", 255, 255, 0),
            ("Green", 0, 255, 0),
            ("Cyan", 0, 255, 255),
            ("Blue", 0, 0, 255),
            ("Purple", 128, 0, 255),
            ("Magenta", 255, 0, 255),
            ("Pink", 255, 105, 180),
            ("White", 255, 255, 255),
            ("Warm White", 255, 214, 170),
            ("Off", 0, 0, 0),
        ];
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.set_margin_top(2);
        row.set_margin_bottom(10);
        row.set_margin_start(12);
        row.set_margin_end(12);
        row.set_halign(gtk::Align::Start);

        let mut css = String::new();
        for (i, (_, r, g, b)) in PRESETS.iter().enumerate() {
            css.push_str(&format!(
                ".{id_prefix}-swatch-{i} {{ background-color: rgb({r},{g},{b}); min-width: 22px; min-height: 22px; padding: 0; border-radius: 999px; }}\n"
            ));
        }
        let provider = gtk::CssProvider::new();
        provider.load_from_string(&css);
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        for (i, (name, r, g, b)) in PRESETS.into_iter().enumerate() {
            let swatch = gtk::Button::new();
            swatch.set_tooltip_text(Some(name));
            swatch.add_css_class(&format!("{id_prefix}-swatch-{i}"));
            let target = target.clone();
            swatch.connect_clicked(move |_| {
                target.set_rgba(&gtk::gdk::RGBA::new(
                    r as f32 / 255.0,
                    g as f32 / 255.0,
                    b as f32 / 255.0,
                    1.0,
                ));
            });
            row.append(&swatch);
        }
        row
    }

    let color1 = ColorRow::new("Color", "Primary color for this effect");
    effects_section.add_row(&color1.row);
    let color1_quick = make_quick_colors_row(&color1.button, "c1");
    effects_section.add_row(&color1_quick);

    let color2 = ColorRow::new("Color 2", "Second color (Dual mode only)");
    effects_section.add_row(&color2.row);
    let color2_quick = make_quick_colors_row(&color2.button, "c2");
    effects_section.add_row(&color2_quick);

    // Wire values are 1/2 (clamped in firmware), not 0/1 - the list index needs +1 on send,
    // -1 on restore, same as the color-mode combo above. Confirmed backwards against real
    // hardware (list index 0 -> wire value 1 is actually "Right", not "Left") - labels swapped
    // below to match; the code still always sends list-index+1, only the two strings moved.
    let direction_combo =
        make_combo_row("Direction", "Wave scroll direction", &["Right", "Left"], 0);
    effects_section.add_row(&direction_combo);

    // Shared by Reactive (real range 1-4) and Starlight (real range 1-3, confirmed from source);
    // sending 4 to Starlight just clamps harmlessly in firmware, so one control covers both
    // rather than needing two differently-bounded sliders swapped in and out per effect.
    let speed_slider = SliderRow::new("Speed", "", 1.0, 4.0, 1.0, 2.0);
    speed_slider.add_mark(1.0, Some("1"));
    speed_slider.add_mark(2.0, Some("2"));
    speed_slider.add_mark(3.0, Some("3"));
    speed_slider.add_mark(4.0, Some("4"));
    effects_section.add_row(&speed_slider.container);

    // Wheel is our own software effect (see kbd::effects::Wheel), so its "speed" is just a plain
    // 0-100 percentage sent as-is, no lookup/offset needed.
    // Wire values are the actual percentage (25/50/75/100), but the slider itself shows plain
    // steps 1-4 (1=25%, 2=50%, 3=75%, 4=100%) - same convention as Stars Speed.
    const WHEEL_SPEEDS: [u8; 4] = [25, 50, 75, 100];
    let wheel_speed_slider = SliderRow::new("Wheel Speed", "", 1.0, 4.0, 1.0, 2.0);
    wheel_speed_slider.add_mark(1.0, Some("1"));
    wheel_speed_slider.add_mark(2.0, Some("2"));
    wheel_speed_slider.add_mark(3.0, Some("3"));
    wheel_speed_slider.add_mark(4.0, Some("4"));
    effects_section.add_row(&wheel_speed_slider.container);

    // Sound Bar's own color-mode selector - Static uses color1 below, Rainbow/Intensity ignore it.
    let sound_bar_mode_combo = make_combo_row(
        "Color Mode",
        "Rainbow per column, one static color, or an intensity gradient (blue -> green -> orange -> red/white)",
        &["Rainbow", "Static", "Intensity Gradient"],
        0,
    );
    effects_section.add_row(&sound_bar_mode_combo);

    let sensitivity_slider = SliderRow::new(
        "Sensitivity",
        "How small a jump above the recent volume counts as a real hit/beat",
        1.0,
        200.0,
        1.0,
        100.0,
    );
    effects_section.add_row(&sensitivity_slider.container);

    let decay_slider = SliderRow::new(
        "Decay",
        "How fast a burst falls back to black after peaking - low is a snappy firework, high lingers longer",
        0.0,
        100.0,
        1.0,
        30.0,
    );
    effects_section.add_row(&decay_slider.container);

    let brightness_limit_slider = SliderRow::new(
        "Brightness Limit",
        "Caps the effect's maximum output brightness",
        0.0,
        100.0,
        1.0,
        100.0,
    );
    effects_section.add_row(&brightness_limit_slider.container);

    // Stars is our own software effect (see kbd::effects::Stars) - per-key random colors,
    // re-rolled on a fixed interval. Wire values are 1-4 directly (1=slowest, 4=fastest), so the
    // slider's value is sent as-is, no offset needed.
    let stars_speed_slider = SliderRow::new("Stars Speed", "", 1.0, 4.0, 1.0, 1.0);
    stars_speed_slider.add_mark(1.0, Some("1"));
    stars_speed_slider.add_mark(2.0, Some("2"));
    stars_speed_slider.add_mark(3.0, Some("3"));
    stars_speed_slider.add_mark(4.0, Some("4"));
    effects_section.add_row(&stars_speed_slider.container);

    // Ripple's color mode - wire values 1=Rainbow/2=Static, same as Audio Meter's combo.
    const RIPPLE_MODE_STATIC: u32 = 1;
    let ripple_mode_combo = make_combo_row(
        "Color Mode",
        "One color, or a rainbow that shifts through the color wheel as each ring spreads",
        &["Rainbow", "Static"],
        RIPPLE_MODE_STATIC,
    );
    effects_section.add_row(&ripple_mode_combo);

    // Wire values are 1-4 directly (1=slowest, 4=fastest), same as Stars Speed.
    let ripple_speed_slider = SliderRow::new("Ripple Speed", "", 1.0, 4.0, 1.0, 3.0);
    ripple_speed_slider.add_mark(1.0, Some("1"));
    ripple_speed_slider.add_mark(2.0, Some("2"));
    ripple_speed_slider.add_mark(3.0, Some("3"));
    ripple_speed_slider.add_mark(4.0, Some("4"));
    effects_section.add_row(&ripple_speed_slider.container);

    let apply_visibility = {
        let mode_combo = mode_combo.clone();
        let color1_row = color1.row.clone();
        let color2_row = color2.row.clone();
        let color1_quick = color1_quick.clone();
        let color2_quick = color2_quick.clone();
        let direction_combo = direction_combo.clone();
        let speed_container = speed_slider.container.clone();
        let wheel_speed_container = wheel_speed_slider.container.clone();
        let sound_bar_mode_combo = sound_bar_mode_combo.clone();
        let sensitivity_container = sensitivity_slider.container.clone();
        let decay_container = decay_slider.container.clone();
        let brightness_limit_container = brightness_limit_slider.container.clone();
        let stars_speed_container = stars_speed_slider.container.clone();
        let ripple_mode_combo = ripple_mode_combo.clone();
        let ripple_speed_container = ripple_speed_slider.container.clone();
        move |effect: u32, mode: u32| {
            let ripple_static = ripple_mode_combo.selected() == RIPPLE_MODE_STATIC;
            let (m, c1, c2, dir, speed, wheel, soundbar, stars, ripple) =
                visible_for(effect, mode, ripple_static);
            mode_combo.set_visible(m);
            color1_row.set_visible(c1);
            color1_quick.set_visible(c1);
            color2_row.set_visible(c2);
            color2_quick.set_visible(c2);
            direction_combo.set_visible(dir);
            speed_container.set_visible(speed);
            wheel_speed_container.set_visible(wheel);
            sound_bar_mode_combo.set_visible(soundbar);
            sensitivity_container.set_visible(soundbar);
            decay_container.set_visible(soundbar);
            brightness_limit_container.set_visible(soundbar);
            stars_speed_container.set_visible(stars);
            ripple_mode_combo.set_visible(ripple);
            ripple_speed_container.set_visible(ripple);
        }
    };
    apply_visibility(STATIC, MODE_SINGLE);

    // Restore saved effect selection and parameters from the daemon. Unknown raw ids (e.g.
    // CUSTOMFRAME, which this combo never sends) just leave the default selection in place.
    if let Some((effect_hw_id, params)) = get_standard_effect()
        && let Some(effect_idx) = combo_index_for_hw_id(effect_hw_id)
    {
        effect_combo.set_selected(effect_idx);
        let set_color = |row: &ColorRow, r: u8, g: u8, b: u8| {
            row.button.set_rgba(&gtk::gdk::RGBA::new(
                r as f32 / 255.0,
                g as f32 / 255.0,
                b as f32 / 255.0,
                1.0,
            ));
        };
        let mut restored_mode = MODE_SINGLE;
        match effect_idx {
            STATIC if params.len() >= 3 => set_color(&color1, params[0], params[1], params[2]),
            WAVE if !params.is_empty() => {
                direction_combo.set_selected((params[0] as u32).saturating_sub(1));
            }
            REACTIVE if params.len() >= 4 => {
                speed_slider.scale.set_value(params[0] as f64);
                set_color(&color1, params[1], params[2], params[3]);
            }
            // Breathing/Starlight are variable-length on the wire (see the Apply handler) - the
            // mode/type byte itself says how many more bytes to expect, rather than a fixed shape.
            BREATHING if !params.is_empty() => {
                restored_mode = (params[0] as u32).clamp(MODE_SINGLE, MODE_RANDOM);
                mode_combo.set_selected(restored_mode - 1);
                match restored_mode {
                    MODE_DUAL if params.len() >= 7 => {
                        set_color(&color1, params[1], params[2], params[3]);
                        set_color(&color2, params[4], params[5], params[6]);
                    }
                    MODE_SINGLE if params.len() >= 4 => {
                        set_color(&color1, params[1], params[2], params[3])
                    }
                    _ => {}
                }
            }
            STARLIGHT if !params.is_empty() => {
                restored_mode = (params[0] as u32).clamp(MODE_SINGLE, MODE_RANDOM);
                mode_combo.set_selected(restored_mode - 1);
                if params.len() >= 2 {
                    speed_slider.scale.set_value(params[1] as f64);
                }
                match restored_mode {
                    MODE_DUAL if params.len() >= 8 => {
                        set_color(&color1, params[2], params[3], params[4]);
                        set_color(&color2, params[5], params[6], params[7]);
                    }
                    MODE_SINGLE if params.len() >= 5 => {
                        set_color(&color1, params[2], params[3], params[4])
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        apply_visibility(effect_idx, restored_mode);
    }

    // Re-evaluate visibility whenever the effect or color-mode selection changes
    {
        let mode_combo_ref = mode_combo.clone();
        let apply_visibility = apply_visibility.clone();
        effect_combo.connect_selected_notify(move |c| {
            apply_visibility(c.selected(), mode_combo_ref.selected() + 1);
        });
    }
    {
        let effect_combo_ref = effect_combo.clone();
        let apply_visibility = apply_visibility.clone();
        mode_combo.connect_selected_notify(move |c| {
            apply_visibility(effect_combo_ref.selected(), c.selected() + 1);
        });
    }
    {
        let effect_combo_ref = effect_combo.clone();
        let mode_combo_ref = mode_combo.clone();
        let apply_visibility = apply_visibility.clone();
        ripple_mode_combo.connect_selected_notify(move |_| {
            apply_visibility(effect_combo_ref.selected(), mode_combo_ref.selected() + 1);
        });
    }

    // Apply button
    let button_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    button_box.set_margin_top(12);
    button_box.set_margin_bottom(12);
    button_box.set_margin_start(12);
    button_box.set_margin_end(12);
    button_box.set_halign(gtk::Align::End);

    let apply_button = gtk::Button::with_label("Apply Effect");
    apply_button.add_css_class("suggested-action");
    button_box.append(&apply_button);
    effects_section.add_row(&button_box);

    {
        let effect_ref = effect_combo.clone();
        let mode_ref = mode_combo.clone();
        let direction_ref = direction_combo.clone();
        let speed_ref = speed_slider.scale.clone();
        let wheel_speed_ref = wheel_speed_slider.scale.clone();
        let sound_bar_mode_ref = sound_bar_mode_combo.clone();
        let sensitivity_ref = sensitivity_slider.scale.clone();
        let decay_ref = decay_slider.scale.clone();
        let brightness_limit_ref = brightness_limit_slider.scale.clone();
        let stars_speed_ref = stars_speed_slider.scale.clone();
        let ripple_mode_ref = ripple_mode_combo.clone();
        let ripple_speed_ref = ripple_speed_slider.scale.clone();
        let color1_btn = color1.button.clone();
        let color2_btn = color2.button.clone();
        apply_button.connect_clicked(move |btn| {
            let c1 = color1_btn.rgba();
            let (r1, g1, b1) = (
                (c1.red() * 255.0) as u8,
                (c1.green() * 255.0) as u8,
                (c1.blue() * 255.0) as u8,
            );
            let c2 = color2_btn.rgba();
            let (r2, g2, b2) = (
                (c2.red() * 255.0) as u8,
                (c2.green() * 255.0) as u8,
                (c2.blue() * 255.0) as u8,
            );
            // Wire values, not GTK list indices - see the MODE_*/direction comments above.
            let mode = mode_ref.selected() as u8 + 1;
            let direction = direction_ref.selected() as u8 + 1;
            let speed = speed_ref.value() as u8;
            let effect = effect_ref.selected();

            // Breathing/Starlight are genuinely variable-length on the wire (1/4/7 bytes and
            // 2/5/8 bytes respectively) - there is no fixed-width, zero-padded shape; sending
            // extra trailing bytes for Single/Random was itself part of the original bug.
            let params: Vec<u8> = match effect {
                OFF | SPECTRUM => vec![],
                STATIC => vec![r1, g1, b1],
                WAVE => vec![direction],
                REACTIVE => vec![speed, r1, g1, b1],
                BREATHING => match mode as u32 {
                    MODE_DUAL => vec![mode, r1, g1, b1, r2, g2, b2],
                    MODE_RANDOM => vec![mode],
                    _ => vec![mode, r1, g1, b1],
                },
                STARLIGHT => match mode as u32 {
                    MODE_DUAL => vec![mode, speed, r1, g1, b1, r2, g2, b2],
                    MODE_RANDOM => vec![mode, speed],
                    _ => vec![mode, speed, r1, g1, b1],
                },
                _ => vec![],
            };
            // Wheel is a different report family entirely (see WHEEL's declaration above) - it
            // never goes through SetStandardEffect/EFFECT_NAMES.
            let ok = if effect == WHEEL {
                let wheel_speed = WHEEL_SPEEDS[(wheel_speed_ref.value() as usize).clamp(1, 4) - 1];
                set_wheel_effect(direction, wheel_speed)
            } else if effect == SOUNDBAR {
                // Wire values, same convention as mode/direction above.
                let color_mode = sound_bar_mode_ref.selected() as u8 + 1;
                let sensitivity = sensitivity_ref.value() as u8;
                let decay = decay_ref.value() as u8;
                let brightness = brightness_limit_ref.value() as u8;
                set_sound_bar_effect(color_mode, r1, g1, b1, sensitivity, decay, brightness)
            } else if effect == STARS {
                // Slider value is already the 1-4 wire value directly, no offset needed.
                let stars_speed = stars_speed_ref.value() as u8;
                set_stars_effect(stars_speed)
            } else if effect == RIPPLE {
                let color_mode = ripple_mode_ref.selected() as u8 + 1;
                let ripple_speed = ripple_speed_ref.value() as u8;
                set_ripple_effect(color_mode, r1, g1, b1, ripple_speed)
            } else {
                let name = EFFECT_NAMES[effect as usize];
                set_standard_effect(name, params)
            };

            // Show toast feedback
            if let Some(root) = btn.root()
                && let Some(window) = root.downcast_ref::<adw::ApplicationWindow>()
                && let Some(child) = window.content()
                && let Ok(overlay) = child.downcast::<adw::ToastOverlay>()
            {
                let toast = if ok == Some(true) {
                    adw::Toast::new("Effect applied")
                } else {
                    adw::Toast::new("Failed to apply effect")
                };
                toast.set_timeout(2);
                overlay.add_toast(toast);
            }
        });
    }

    // --- Per-Key Painting ---
    // Bypasses SetStandardEffect entirely: SetCustomKey/FillCustomFrame drive the daemon's
    // EffectManager::set_custom_key/fill_custom directly (per-key custom frame HID reports),
    // which requires the device's "per_key_rgb" feature flag to be on (see laptops.json).
    const PAINT_ROWS: usize = 6;
    const PAINT_COLS: usize = 16;
    const PAINT_KEYS: usize = PAINT_ROWS * PAINT_COLS;

    // Best-effort labels for a standard non-numpad US Blade layout. There is no authoritative
    // per-key row/col map for this exact model anywhere (checked OpenRazer's driver and daemon
    // source - only a desktop 22-column BlackWidow layout is documented there), so this is a
    // reconstruction from the well-known general layout, not a verified source. Cells marked "?"
    // are ones I have no real basis to guess (mainly the top-right cluster and a few row-end
    // slots) - click one on the real keyboard and let me know what it actually is so it can be
    // corrected, rather than trusting an unconfirmed label.
    const KEY_LABELS: [[&str; 16]; 6] = [
        [
            "Esc", "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
            "PrtSc?", "Ins?", "Pwr?",
        ],
        [
            "`", "1", "2", "3", "4", "5", "6", "7", "8", "9", "0", "-", "=", "Bksp", "?", "?",
        ],
        [
            "Tab", "Q", "W", "E", "R", "T", "Y", "U", "I", "O", "P", "[", "]", "\\", "?", "?",
        ],
        [
            "Caps", "A", "S", "D", "F", "G", "H", "J", "K", "L", ";", "'", "Enter", "?", "?", "?",
        ],
        [
            "Shift", "Z", "X", "C", "V", "B", "N", "M", ",", ".", "/", "Shift", "?", "Up?", "?",
            "?",
        ],
        [
            "Ctrl", "Win", "Alt", "Space", "Space", "Space", "Space", "Space", "AltGr", "Fn",
            "Ctx", "Ctrl", "Left?", "Down?", "Right?", "?",
        ],
    ];

    let paint_section = settings_page.add_section(Some("Per-Key Painting"));

    let paint_color = ColorRow::new("Paint Color", "Color used when you click a key");
    paint_section.add_row(&paint_color.row);
    let paint_color_quick = make_quick_colors_row(&paint_color.button, "paint");
    paint_section.add_row(&paint_color_quick);

    // Row-major order (index = row * PAINT_COLS + col), matching board::KeyboardData::get_curr_state().
    let key_colors: Rc<RefCell<[(u8, u8, u8); PAINT_KEYS]>> =
        Rc::new(RefCell::new([(0, 0, 0); PAINT_KEYS]));
    if let Some(rgb) = get_keyboard_rgb() {
        let mut kc = key_colors.borrow_mut();
        for i in 0..PAINT_KEYS.min(rgb.len() / 3) {
            kc[i] = (rgb[i * 3], rgb[i * 3 + 1], rgb[i * 3 + 2]);
        }
    }

    let paint_css_provider = gtk::CssProvider::new();
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &paint_css_provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    fn rebuild_paint_css(provider: &gtk::CssProvider, colors: &[(u8, u8, u8); PAINT_KEYS]) {
        let mut css = String::from(
            ".paint-key label { font-size: 8px; color: #fff; text-shadow: 0 0 2px #000, 0 0 2px #000; }\n",
        );
        for (i, (r, g, b)) in colors.iter().enumerate() {
            css.push_str(&format!(
                ".paint-key-{i} {{ background-color: rgb({r},{g},{b}); min-width: 30px; min-height: 24px; padding: 0; border-radius: 4px; }}\n"
            ));
        }
        provider.load_from_string(&css);
    }
    rebuild_paint_css(&paint_css_provider, &key_colors.borrow());

    let paint_grid = gtk::Grid::new();
    paint_grid.set_row_spacing(3);
    paint_grid.set_column_spacing(3);
    paint_grid.set_margin_top(8);
    paint_grid.set_margin_bottom(8);
    paint_grid.set_margin_start(12);
    paint_grid.set_margin_end(12);
    paint_grid.set_halign(gtk::Align::Start);

    for (row, labels) in KEY_LABELS.iter().enumerate() {
        for (col, &label) in labels.iter().enumerate() {
            let index = row * PAINT_COLS + col;
            let cell = gtk::Button::with_label(label);
            cell.add_css_class("paint-key");
            cell.add_css_class(&format!("paint-key-{index}"));
            cell.set_tooltip_text(Some(if label == "?" {
                "Unlabeled key position"
            } else {
                label
            }));
            let key_colors = key_colors.clone();
            let paint_color_btn = paint_color.button.clone();
            let provider = paint_css_provider.clone();
            cell.connect_clicked(move |_| {
                let c = paint_color_btn.rgba();
                let (r, g, b) = (
                    (c.red() * 255.0) as u8,
                    (c.green() * 255.0) as u8,
                    (c.blue() * 255.0) as u8,
                );
                key_colors.borrow_mut()[index] = (r, g, b);
                rebuild_paint_css(&provider, &key_colors.borrow());
                set_custom_key(index as u8, r, g, b);
            });
            paint_grid.attach(&cell, col as i32, row as i32, 1, 1);
        }
    }
    paint_section.add_row(&paint_grid);

    let paint_btn_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    paint_btn_box.set_halign(gtk::Align::End);
    paint_btn_box.set_margin_top(8);
    paint_btn_box.set_margin_bottom(12);
    paint_btn_box.set_margin_start(12);
    paint_btn_box.set_margin_end(12);

    let fill_all_btn = gtk::Button::with_label("Fill All");
    let random_all_btn = gtk::Button::with_label("Random All");
    let clear_all_btn = gtk::Button::with_label("Clear All");
    paint_btn_box.append(&fill_all_btn);
    paint_btn_box.append(&random_all_btn);
    paint_btn_box.append(&clear_all_btn);
    paint_section.add_row(&paint_btn_box);

    {
        let key_colors = key_colors.clone();
        let provider = paint_css_provider.clone();
        let paint_color_btn = paint_color.button.clone();
        fill_all_btn.connect_clicked(move |_| {
            let c = paint_color_btn.rgba();
            let (r, g, b) = (
                (c.red() * 255.0) as u8,
                (c.green() * 255.0) as u8,
                (c.blue() * 255.0) as u8,
            );
            *key_colors.borrow_mut() = [(r, g, b); PAINT_KEYS];
            rebuild_paint_css(&provider, &key_colors.borrow());
            fill_custom_frame(r, g, b);
        });
    }
    {
        let key_colors = key_colors.clone();
        let provider = paint_css_provider.clone();
        clear_all_btn.connect_clicked(move |_| {
            *key_colors.borrow_mut() = [(0, 0, 0); PAINT_KEYS];
            rebuild_paint_css(&provider, &key_colors.borrow());
            fill_custom_frame(0, 0, 0);
        });
    }
    {
        // Colors are generated server-side (daemon has no client-visible RNG state), so refetch
        // the real result via GetKeyboardRGB rather than guessing colors here.
        let key_colors = key_colors.clone();
        let provider = paint_css_provider.clone();
        random_all_btn.connect_clicked(move |_| {
            randomize_custom_frame();
            if let Some(rgb) = get_keyboard_rgb() {
                let mut kc = key_colors.borrow_mut();
                for i in 0..PAINT_KEYS.min(rgb.len() / 3) {
                    kc[i] = (rgb[i * 3], rgb[i * 3 + 1], rgb[i * 3 + 2]);
                }
                rebuild_paint_css(&provider, &kc);
            }
        });
    }

    // --- Callbacks for AC/Battery toggle (brightness + logo only) ---

    let refresh = {
        let is_ac = is_ac.clone();
        let refreshing = refreshing.clone();
        let brightness_scale = brightness_slider.scale.clone();
        let logo_combo = logo_combo.clone();
        move || {
            refreshing.set(true);
            let ac = is_ac.get();
            let br = get_brightness(ac).unwrap_or(100);
            brightness_scale.set_value(br as f64);
            if let Some(ref lc) = logo_combo {
                let logo = get_logo(ac).unwrap_or(1);
                lc.set_selected(logo as u32);
            }
            refreshing.set(false);
        }
    };

    // Hook toggle buttons for refresh
    {
        let first_child = toggle_box.first_child();
        if let Some(ac_btn) = first_child
            && let Ok(tb) = ac_btn.downcast::<gtk::ToggleButton>()
        {
            let refresh = refresh.clone();
            tb.connect_toggled(move |btn| {
                if btn.is_active() {
                    refresh();
                }
            });
        }
        let last_child = toggle_box.last_child();
        if let Some(bat_btn) = last_child
            && let Ok(tb) = bat_btn.downcast::<gtk::ToggleButton>()
        {
            let refresh = refresh.clone();
            tb.connect_toggled(move |btn| {
                if btn.is_active() {
                    refresh();
                }
            });
        }
    }

    // Brightness change
    {
        let is_ac = is_ac.clone();
        let refreshing = refreshing.clone();
        brightness_slider.scale.connect_value_changed(move |sc| {
            if refreshing.get() {
                return;
            }
            let ac = is_ac.get();
            set_brightness(ac, sc.value() as u8);
        });
    }

    // Logo change
    if let Some(ref lc) = logo_combo {
        let is_ac = is_ac.clone();
        let refreshing = refreshing.clone();
        lc.connect_selected_notify(move |c| {
            if refreshing.get() {
                return;
            }
            let ac = is_ac.get();
            let logo = c.selected() as u8;
            set_logo(ac, logo);
        });
    }

    // Live-sync: poll daemon every 2s so widget changes appear in GUI
    {
        let refresh = refresh.clone();
        glib::timeout_add_local(Duration::from_secs(2), move || {
            refresh();
            glib::ControlFlow::Continue
        });
    }

    settings_page
}

// ---------------------------------------------------------------------------
// Battery page
// ---------------------------------------------------------------------------

fn make_battery_page() -> SettingsPage {
    let page = SettingsPage::new();

    let bho = get_bho();

    if let Some(bho) = bho {
        let refreshing = Rc::new(Cell::new(false));
        let section = page.add_section(Some("Battery Health Optimizer"));

        let bho_switch = make_switch_row(
            "Limit Charging",
            "Cap maximum charge to extend battery lifespan",
            bho.0,
        );
        section.add_row(&bho_switch);

        let bho_slider = SliderRow::new(
            "Charge Limit",
            "Maximum battery charge level (%)",
            50.0,
            80.0,
            5.0,
            bho.1 as f64,
        );
        bho_slider.add_mark(50.0, Some("50%"));
        bho_slider.add_mark(65.0, Some("65%"));
        bho_slider.add_mark(80.0, Some("80%"));
        bho_slider.scale.set_sensitive(bho.0);
        section.add_row(&bho_slider.container);

        {
            let bho_switch_ref = bho_switch.clone();
            let refreshing = refreshing.clone();
            bho_slider.scale.connect_value_changed(move |sc| {
                if refreshing.get() {
                    return;
                }
                let is_on = bho_switch_ref.is_active();
                let threshold = sc.value() as u8;
                set_bho(is_on, threshold);
            });
        }

        {
            let refreshing = refreshing.clone();
            let scale_ref = bho_slider.scale.clone();
            bho_switch.connect_active_notify(glib::clone!(
                #[weak]
                scale_ref,
                move |sw| {
                    if refreshing.get() {
                        return;
                    }
                    let state = sw.is_active();
                    let threshold = scale_ref.value() as u8;
                    set_bho(state, threshold);
                    scale_ref.set_sensitive(state);
                }
            ));
        }

        // Live-sync: poll daemon every 2s so widget changes appear in GUI
        {
            let bho_switch = bho_switch.clone();
            let bho_scale = bho_slider.scale.clone();
            glib::timeout_add_local(Duration::from_secs(2), move || {
                if let Some((is_on, threshold)) = get_bho() {
                    refreshing.set(true);
                    bho_switch.set_active(is_on);
                    bho_scale.set_value(threshold as f64);
                    bho_scale.set_sensitive(is_on);
                    refreshing.set(false);
                }
                glib::ControlFlow::Continue
            });
        }
    } else {
        let status = adw::StatusPage::new();
        status.set_icon_name(Some("battery-symbolic"));
        status.set_title("Not Available");
        status.set_description(Some(
            "Battery health optimizer is not supported on this device.",
        ));
        page.page.add(&adw::PreferencesGroup::new());
        let section = page.add_section(None);
        section.add_row(&status);
    }

    page
}

fn gpu_mode_description(index: u32) -> &'static str {
    match index {
        0 => "Both GPUs active, apps choose which to use",
        1 => "Only integrated GPU, maximum battery life",
        2 => "Only NVIDIA GPU, maximum performance",
        _ => "",
    }
}

// ---------------------------------------------------------------------------
// About page
// ---------------------------------------------------------------------------

fn make_about_page(device: SupportedDevice) -> SettingsPage {
    let page = SettingsPage::new();

    // Application Info Section
    let section = page.add_section(Some("Application"));

    let app_name = gtk::Label::new(Some("Razer Control"));
    app_name.add_css_class("title-2");
    let row = SettingsRow::new("Name", &app_name);
    section.add_row(&row.row);

    let version_label = gtk::Label::new(Some(&format!("v{}", env!("CARGO_PKG_VERSION"))));
    let row = SettingsRow::new("Version", &version_label);
    section.add_row(&row.row);

    // Device Information Section
    let section = page.add_section(Some("Device Information"));

    let name_label = gtk::Label::new(Some(&device.name));
    name_label.set_wrap(true);
    let row = SettingsRow::new("Model", &name_label);
    row.set_subtitle("Detected Razer laptop model");
    section.add_row(&row.row);

    let features = device.features.join(", ");
    let features_label = gtk::Label::new(Some(&features));
    features_label.set_wrap(true);
    let row = SettingsRow::new("Features", &features_label);
    row.set_subtitle("Supported hardware capabilities");
    section.add_row(&row.row);

    let fan_min = device.fan.get(0).unwrap_or(&0);
    let fan_max = device.fan.get(1).unwrap_or(&5000);
    let fan_range = format!("{} - {} RPM", fan_min, fan_max);
    let fan_label = gtk::Label::new(Some(&fan_range));
    let row = SettingsRow::new("Fan Range", &fan_label);
    row.set_subtitle("Minimum to maximum fan speed");
    section.add_row(&row.row);

    // About Section
    let section = page.add_section(Some("About"));

    let desc_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    desc_box.set_margin_top(12);
    desc_box.set_margin_bottom(12);
    desc_box.set_margin_start(12);
    desc_box.set_margin_end(12);

    let description = gtk::Label::new(Some(
        "Open-source control center for Razer laptops on Linux.\n\
        Manage power profiles, fan speeds, keyboard lighting, and more.\n\n\
        \u{26A0}\u{FE0F} Tested on: Fedora Linux\n\
        Should work on Ubuntu and similar distributions.\n\
        If issues occur, please report them on GitHub.",
    ));
    description.set_wrap(true);
    description.set_justify(gtk::Justification::Center);
    description.add_css_class("dim-label");
    desc_box.append(&description);
    section.add_row(&desc_box);
    page
}
