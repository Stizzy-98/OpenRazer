use libc::umask;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};

/// Daemon socket path, inside the user's private runtime directory (mode 0700). Never falls back
/// to a shared directory like /tmp, where any local user could reach or squat on the socket.
pub fn socket_path() -> String {
    let dir = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() }));
    format!("{}/razercontrol-socket", dir)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GpuInfo {
    pub name: String,
    pub pci_slot: String,
    pub driver: String,
    pub gpu_type: String,
    pub runtime_status: String,
}

#[derive(Serialize, Deserialize, Debug)]
/// Represents data sent TO the daemon
pub enum DaemonCommand {
    SetFanSpeed {
        ac: usize,
        rpm: i32,
    }, // Fan speed
    GetFanSpeed {
        ac: usize,
    }, // Get (Fan speed)
    SetPowerMode {
        ac: usize,
        pwr: u8,
        cpu: u8,
        gpu: u8,
    }, // Power mode
    GetPwrLevel {
        ac: usize,
    }, // Get (Power mode)
    GetCPUBoost {
        ac: usize,
    }, // Get (CPU boost)
    GetGPUBoost {
        ac: usize,
    }, // Get (GPU boost)
    SetLogoLedState {
        ac: usize,
        logo_state: u8,
    },
    GetLogoLedState {
        ac: usize,
    },
    GetKeyboardRGB {
        layer: i32,
    }, // Layer ID
    SetEffect {
        name: String,
        params: Vec<u8>,
    }, // Set keyboard colour
    SetStandardEffect {
        name: String,
        params: Vec<u8>,
    }, // Set keyboard colour
    SetCustomKey {
        index: u8,
        r: u8,
        g: u8,
        b: u8,
    }, // Paint a single key (per-key custom frame)
    FillCustomFrame {
        r: u8,
        g: u8,
        b: u8,
    }, // Fill every key (per-key custom frame)
    RandomizeCustomFrame,
    SetWheelEffect {
        direction: u8,
        speed: u8,
    },
    SetSoundBarEffect {
        color_mode: u8,
        r: u8,
        g: u8,
        b: u8,
        sensitivity: u8, // 1-200, 100 = normal (percent-style gain on the silence gate)
        decay: u8,       // 0-100, higher lingers/fades slower
        brightness: u8,  // 0-100, output brightness cap
    }, // Audio-reactive equalizer effect
    SetStarsEffect {
        speed: u8, // 1-4: 1=slowest (1.0s between re-randomizes), 4=fastest (0.25s)
    }, // Per-key random colors re-rolled on an interval
    SetRippleEffect {
        color_mode: u8, // 1=Rainbow, 2=Static
        r: u8,
        g: u8,
        b: u8,
        speed: u8, // 1-4: 1=slowest, 4=fastest
    }, // Rings of light spreading out from each key press
    SetBrightness {
        ac: usize,
        val: u8,
    },
    SetIdle {
        ac: usize,
        val: u32,
    },
    GetBrightness {
        ac: usize,
    },
    SetSync {
        sync: bool,
    },
    GetSync(),
    SetBatteryHealthOptimizer {
        is_on: bool,
        threshold: u8,
    },
    GetBatteryHealthOptimizer(),
    GetDeviceName,
    GetActualFanRpm,
    GetStandardEffect,
    GetGpuStatus,
    SetDgpuRuntimePM {
        enabled: bool,
    },
    SetGpuMode {
        mode: String,
    },
}

#[derive(Serialize, Deserialize, Debug)]
/// Represents data sent back from Daemon after it receives
/// a command.
pub enum DaemonResponse {
    SetFanSpeed {
        result: bool,
    }, // Response
    GetFanSpeed {
        rpm: i32,
    }, // Get (Fan speed)
    SetPowerMode {
        result: bool,
    }, // Response
    GetPwrLevel {
        pwr: u8,
    }, // Get (Power mode)
    GetCPUBoost {
        cpu: u8,
    }, // Get (CPU boost)
    GetGPUBoost {
        gpu: u8,
    }, // Get (GPU boost)
    SetLogoLedState {
        result: bool,
    },
    GetLogoLedState {
        logo_state: u8,
    },
    GetKeyboardRGB {
        layer: i32,
        rgbdata: Vec<u8>,
    }, // Response (RGB) of 90 keys
    SetEffect {
        result: bool,
    }, // Set keyboard colour
    SetStandardEffect {
        result: bool,
    }, // Set keyboard colour
    SetCustomKey {
        result: bool,
    },
    FillCustomFrame {
        result: bool,
    },
    RandomizeCustomFrame {
        result: bool,
    },
    SetWheelEffect {
        result: bool,
    },
    SetSoundBarEffect {
        result: bool,
    },
    SetStarsEffect {
        result: bool,
    },
    SetRippleEffect {
        result: bool,
    },
    SetBrightness {
        result: bool,
    },
    SetIdle {
        result: bool,
    },
    GetBrightness {
        result: u8,
    },
    SetSync {
        result: bool,
    },
    GetSync {
        sync: bool,
    },
    SetBatteryHealthOptimizer {
        result: bool,
    },
    GetBatteryHealthOptimizer {
        is_on: bool,
        threshold: u8,
    },
    GetDeviceName {
        name: String,
    },
    GetActualFanRpm {
        rpm: i32,
    },
    GetStandardEffect {
        effect: u8,
        params: Vec<u8>,
    },
    GetGpuStatus {
        gpus: Vec<GpuInfo>,
        dgpu_runtime_pm: bool,
        envycontrol_mode: String,
        envycontrol_available: bool,
    },
    SetDgpuRuntimePM {
        result: bool,
    },
    SetGpuMode {
        result: bool,
        message: String,
    },
}

pub fn bind() -> Option<UnixStream> {
    UnixStream::connect(socket_path()).ok()
}

/// We use this from the app, but it should replace bind
pub fn try_bind() -> std::io::Result<UnixStream> {
    UnixStream::connect(socket_path())
}

pub fn create() -> Option<UnixListener> {
    let path = socket_path();
    if std::fs::metadata(&path).is_ok() {
        // Socket file exists — check if a daemon is actually listening
        if UnixStream::connect(&path).is_ok() {
            eprintln!(
                "UNIX Socket already exists and a daemon is responding. Is another daemon running?"
            );
            return None;
        }
        // Stale socket from a previous crash — remove it
        eprintln!("Removing stale socket file");
        if std::fs::remove_file(&path).is_err() {
            eprintln!("Could not remove stale socket file");
            return None;
        }
    }
    // Owner-only (0600-equivalent): the daemon, GUI, and widget helper all run as the same user.
    let old_umask = unsafe { umask(0o077) };
    let result = UnixListener::bind(&path);
    unsafe { umask(old_umask) };
    match result {
        Ok(listener) => Some(listener),
        Err(e) => {
            eprintln!("Failed to bind socket: {}", e);
            None
        }
    }
}

pub fn send_to_daemon(command: DaemonCommand, mut sock: UnixStream) -> Option<DaemonResponse> {
    // Prevent blocking the GTK main thread forever if daemon is unresponsive
    let timeout = Some(std::time::Duration::from_secs(5));
    let _ = sock.set_read_timeout(timeout);
    let _ = sock.set_write_timeout(timeout);

    if let Ok(encoded) = bincode::serialize(&command) {
        if sock.write_all(&encoded).is_ok() {
            // Signal request EOF to daemon so it can read the full command.
            let _ = sock.shutdown(Shutdown::Write);

            let mut response = Vec::new();
            return match sock.read_to_end(&mut response) {
                Ok(readed) if readed > 0 => read_from_socked_resp(&response),
                Ok(_) => {
                    eprintln!("No response from daemon");
                    None
                }
                Err(error) => {
                    eprintln!("Read failed: {error}");
                    None
                }
            };
        } else {
            eprintln!("Socket write failed!");
        }
    }
    None
}

/// Deserializes incomming bytes in order to return
/// a `DaemonResponse`. None is returned if deserializing failed
fn read_from_socked_resp(bytes: &[u8]) -> Option<DaemonResponse> {
    match bincode::deserialize::<DaemonResponse>(bytes) {
        Ok(res) => {
            println!("RES: {:?}", res);
            Some(res)
        }
        Err(e) => {
            println!("RES ERROR: {}", e);
            None
        }
    }
}

/// Deserializes incomming bytes in order to return
/// a `DaemonCommand`. None is returned if deserializing failed
pub fn read_from_socket_req(bytes: &[u8]) -> Option<DaemonCommand> {
    match bincode::deserialize::<DaemonCommand>(bytes) {
        Ok(res) => {
            println!("REQ: {:?}", res);
            Some(res)
        }
        Err(e) => {
            println!("REQ ERROR: {}", e);
            None
        }
    }
}
