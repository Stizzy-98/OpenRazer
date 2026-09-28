use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time;

use dbus::blocking::Connection;
use dbus::{Message, arg};
use lazy_static::lazy_static;
use log::*;
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

mod audio;
mod battery;
use service::comms;
mod config;
mod dbus_mutter_displayconfig;
mod dbus_mutter_idlemonitor;
mod device;
mod gpu;
mod kbd;
mod keys;
mod login1;
mod screensaver;

use crate::kbd::Effect;

lazy_static! {
    static ref EFFECT_MANAGER: Mutex<kbd::EffectManager> = Mutex::new(kbd::EffectManager::new());
    // static ref CONFIG: Mutex<config::Configuration> = {
        // match config::Configuration::read_from_config() {
            // Ok(c) => Mutex::new(c),
            // Err(_) => Mutex::new(config::Configuration::new()),
        // }
    // };
    static ref DEV_MANAGER: Mutex<device::DeviceManager> = {
        match device::DeviceManager::read_laptops_file() {
            Ok(c) => Mutex::new(c),
            Err(_) => Mutex::new(device::DeviceManager::new()),
        }
    };
    // Holds the live audio-capture subprocess/thread while Sound Bar is active. None means no
    // capture is running - stopping capture is just replacing this with None, which drops the
    // AudioCapture and (via its Drop impl) kills the parec child.
    static ref AUDIO: Mutex<Option<audio::AudioCapture>> = Mutex::new(None);
    // Sensitivity/decay, shared with whatever capture thread is currently running. Updated in
    // place on every SetSoundBarEffect call (even if capture is already running) so adjusting
    // these sliders takes effect live without needing to restart the parec subprocess.
    static ref AUDIO_CONFIG: std::sync::Arc<Mutex<audio::AudioConfig>> =
        std::sync::Arc::new(Mutex::new(audio::AudioConfig::default()));
    // Holds the key-press watcher while Ripple is active; None means the keyboard's input
    // reports aren't being read at all.
    static ref KEYS: Mutex<Option<keys::KeyCapture>> = Mutex::new(None);
}

/// Args of a saved layer named `name`, if effects.json has one.
fn saved_effect_args(json: &serde_json::Value, name: &str) -> Option<Vec<u8>> {
    json["effects"].as_array()?.iter().find_map(|e| {
        (e["name"] == name)
            .then(|| serde_json::from_value(e["args"].clone()).ok())
            .flatten()
    })
}

/// Audio Meter can't be rebuilt by `EffectLayer::from_save` like the other layers - it needs a
/// live `AudioCapture` feeding it - so startup restores it here. Sensitivity/decay aren't part of
/// the saved args (they live in AUDIO_CONFIG), so they come back at their defaults.
fn restore_audio_meter(args: Vec<u8>) {
    // Same lock order as the SetSoundBarEffect handler (effect manager, then audio).
    if let Ok(mut k) = EFFECT_MANAGER.lock()
        && let Ok(mut a) = AUDIO.lock()
    {
        *a = audio::AudioCapture::start(AUDIO_CONFIG.clone());
        if let Some(capture) = a.as_ref() {
            k.push_effect(
                kbd::effects::SoundBar::new_live(args, capture.levels.clone()),
                [true; kbd::TOTAL_KEYS],
            );
        }
    }
}

/// Ripple can't be rebuilt by `EffectLayer::from_save` either - it needs a live `KeyCapture`.
fn restore_ripple(args: Vec<u8>, pid: u16) {
    // Same lock order as the SetRippleEffect handler (effect manager, then keys).
    if let Ok(mut k) = EFFECT_MANAGER.lock()
        && let Ok(mut keys) = KEYS.lock()
    {
        *keys = keys::KeyCapture::start(pid);
        if let Some(capture) = keys.as_ref() {
            k.push_effect(
                kbd::effects::Ripple::new_live(args, capture.presses.clone()),
                [true; kbd::TOTAL_KEYS],
            );
        }
    }
}

/// Stops any running audio capture - called whenever a different effect/paint action takes over,
/// so Sound Bar's parec subprocess doesn't keep running in the background after switching away.
fn stop_audio_capture() {
    if let Ok(mut a) = AUDIO.lock() {
        *a = None;
    }
}

/// Stops watching key presses once Ripple is no longer the active effect, so the keyboard's input
/// reports are only read while they're actually needed.
fn stop_key_capture() {
    if let Ok(mut k) = KEYS.lock() {
        *k = None;
    }
}

/// Stops every live input (audio and key presses) before a non-live effect takes over.
fn stop_live_inputs() {
    stop_audio_capture();
    stop_key_capture();
}

// Main function for daemon
fn main() {
    setup_panic_hook();
    init_logging();

    if let Ok(mut d) = DEV_MANAGER.lock() {
        d.discover_devices();
        if let Some(laptop) = d.get_device() {
            println!("supported device: {:?}", laptop.get_name());
        } else {
            println!("no supported device found");
            std::process::exit(1);
        }
    } else {
        println!("error loading supported devices");
        std::process::exit(1);
    }

    if let Ok(mut d) = DEV_MANAGER.lock() {
        let dbus_system = match Connection::new_system() {
            Ok(conn) => conn,
            Err(e) => {
                eprintln!("Failed to connect to D-Bus system bus: {}", e);
                std::process::exit(1);
            }
        };
        let proxy_ac = dbus_system.with_proxy(
            "org.freedesktop.UPower",
            "/org/freedesktop/UPower/devices/line_power_AC0",
            time::Duration::from_millis(5000),
        );
        use battery::OrgFreedesktopUPowerDevice;
        if let Ok(online) = proxy_ac.online() {
            println!("Online AC0: {:?}", online);
            d.set_ac_state(online);
            d.restore_standard_effect();
            d.restore_bho();
            // Only load per-key RGB effects if device supports custom frames.
            // Sending custom frame HID reports to unsupported devices can
            // overwhelm the USB/HID subsystem and trigger kernel panics.
            if d.device_has_feature("per_key_rgb") {
                if let Ok(json) = config::Configuration::read_effects_file() {
                    let audio_meter_args = saved_effect_args(&json, "Audio Meter");
                    let ripple_args = saved_effect_args(&json, "Ripple");
                    if let Ok(mut mgr) = EFFECT_MANAGER.lock() {
                        mgr.load_from_save(json);
                    }
                    if let Some(args) = audio_meter_args {
                        restore_audio_meter(args);
                    }
                    if let (Some(args), Some(laptop)) = (ripple_args, d.get_device()) {
                        restore_ripple(args, laptop.get_pid());
                    }
                } else {
                    println!("No effects save, creating a new one");
                    if let Ok(mut mgr) = EFFECT_MANAGER.lock() {
                        mgr.push_effect(
                            kbd::effects::Static::new(vec![0, 255, 0]),
                            [true; kbd::TOTAL_KEYS],
                        );
                    }
                }
            } else {
                println!("Device does not support per-key RGB, skipping keyboard effects");
            }
        } else {
            println!("error getting current power state");
            std::process::exit(1);
        }
    }

    // Only run the keyboard animation loop if the device supports per-key RGB.
    // Sending custom frame reports to unsupported devices causes kernel panics.
    if let Ok(d) = DEV_MANAGER.lock() {
        if d.device_has_feature("per_key_rgb") {
            start_keyboard_animator_task();
        } else {
            println!("Keyboard animation disabled (device has no per_key_rgb)");
        }
    }
    start_screensaver_monitor_task();
    start_battery_monitor_task();
    start_update_watch_task();
    let clean_thread = start_shutdown_task();

    if let Some(listener) = comms::create() {
        for stream in listener.incoming() {
            let stream = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };
            handle_data(stream);
        }
    } else {
        eprintln!("Could not create Unix socket!");
        std::process::exit(1);
    }
    clean_thread.join().unwrap();
}

/// Installs a custom panic hook to perform cleanup when the daemon crashes
fn setup_panic_hook() {
    let default_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        error!("Something went wrong! Removing the socket path");
        let _ = std::fs::remove_file(comms::socket_path());
        default_panic_hook(info);
    }));
}

fn init_logging() {
    let mut builder = env_logger::Builder::from_default_env();
    builder.target(env_logger::Target::Stderr);
    builder.filter_level(log::LevelFilter::Info);
    builder.format_timestamp_millis();
    builder.parse_env("RAZER_LAPTOP_CONTROL_LOG");
    builder.init();
}

/// Handles keyboard animations
pub fn start_keyboard_animator_task() -> JoinHandle<()> {
    // Start the keyboard animator thread,
    thread::spawn(|| {
        loop {
            if let Ok(mut dev) = DEV_MANAGER.lock()
                && let Some(laptop) = dev.get_device()
                && let Ok(mut mgr) = EFFECT_MANAGER.lock()
            {
                mgr.update(laptop);
            }
            thread::sleep(std::time::Duration::from_millis(kbd::ANIMATION_SLEEP_MS));
        }
    })
}

fn start_screensaver_monitor_task() -> JoinHandle<()> {
    thread::spawn(move || {
        loop {
            let dbus_session = match Connection::new_session() {
                Ok(conn) => conn,
                Err(e) => {
                    eprintln!(
                        "Screensaver monitor: D-Bus session unavailable ({}), retrying in 5s",
                        e
                    );
                    thread::sleep(time::Duration::from_secs(5));
                    continue;
                }
            };
            let proxy = dbus_session.with_proxy(
                "org.gnome.Mutter.DisplayConfig",
                "/org/gnome/Mutter/DisplayConfig",
                time::Duration::from_millis(5000),
            );
            let _id = proxy.match_signal(
                |h: dbus_mutter_displayconfig::OrgFreedesktopDBusPropertiesPropertiesChanged,
                 _: &Connection,
                 _: &Message| {
                    let online: Option<&i32> =
                        arg::prop_cast(&h.changed_properties, "PowerSaveMode");
                    if let Some(online) = online {
                        if *online == 3 {
                            if let Ok(mut d) = DEV_MANAGER.lock() {
                                d.light_off();
                            }
                        } else if *online == 0
                            && let Ok(mut d) = DEV_MANAGER.lock()
                        {
                            d.restore_light();
                        }
                    }
                    true
                },
            );
            let proxy_idle = dbus_session.with_proxy(
                "org.gnome.Mutter.IdleMonitor",
                "/org/gnome/Mutter/IdleMonitor/Core",
                time::Duration::from_millis(5000),
            );
            let _id = proxy_idle.match_signal(
                |h: dbus_mutter_idlemonitor::OrgGnomeMutterIdleMonitorWatchFired,
                 _: &Connection,
                 _: &Message| {
                    if let Ok(mut d) = DEV_MANAGER.lock() {
                        if d.idle_id == h.id {
                            println!("idle trigger {:?}", h.id);
                            d.light_off();
                        } else if d.active_id == h.id {
                            println!("active trigger {:?}", h.id);
                            d.restore_light();
                        }
                    }
                    true
                },
            );
            let proxy = dbus_session.with_proxy(
                "org.freedesktop.ScreenSaver",
                "/org/freedesktop/ScreenSaver",
                time::Duration::from_millis(5000),
            );
            let _id = proxy.match_signal(
                |h: screensaver::OrgFreedesktopScreenSaverActiveChanged,
                 _: &Connection,
                 _: &Message| {
                    println!("ActiveChanged {:?}", h.arg0);
                    if let Ok(mut d) = DEV_MANAGER.lock() {
                        if h.arg0 {
                            d.light_off();
                        } else {
                            d.restore_light();
                        }
                    }
                    true
                },
            );

            if let Ok(mut d) = DEV_MANAGER.lock() {
                d.idle_id = 0;
                d.active_id = 0;
                d.change_idle = true;
                d.add_idle_watch(&proxy_idle);
            }

            loop {
                match dbus_session.process(time::Duration::from_millis(1000)) {
                    Ok(res) => {
                        if res && let Ok(mut d) = DEV_MANAGER.lock() {
                            d.add_active_watch(&proxy_idle);
                        }
                        if let Ok(mut d) = DEV_MANAGER.lock() {
                            d.add_idle_watch(&proxy_idle);
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "Screensaver monitor: D-Bus session disconnected ({}), reconnecting...",
                            e
                        );
                        thread::sleep(time::Duration::from_secs(2));
                        break;
                    }
                }
            }
        }
    })
}

fn start_battery_monitor_task() -> JoinHandle<()> {
    thread::spawn(move || {
        let dbus_system = match Connection::new_system() {
            Ok(conn) => conn,
            Err(e) => {
                eprintln!(
                    "Battery monitor: D-Bus system unavailable ({}), skipping",
                    e
                );
                return;
            }
        };
        let proxy_ac = dbus_system.with_proxy(
            "org.freedesktop.UPower",
            "/org/freedesktop/UPower/devices/line_power_AC0",
            time::Duration::from_millis(5000),
        );
        let _id = proxy_ac.match_signal(
            |h: battery::OrgFreedesktopDBusPropertiesPropertiesChanged,
             _: &Connection,
             _: &Message| {
                let online: Option<&bool> = arg::prop_cast(&h.changed_properties, "Online");
                if let Some(online) = online {
                    println!("Online AC0: {:?}", online);
                    if let Ok(mut d) = DEV_MANAGER.lock() {
                        d.set_ac_state(*online);
                    }
                }
                true
            },
        );

        let proxy_battery = dbus_system.with_proxy(
            "org.freedesktop.UPower",
            "/org/freedesktop/UPower/devices/battery_BAT0",
            time::Duration::from_millis(5000),
        );
        // use battery::OrgFreedesktopUPowerDevice;
        // if let Ok(perc) = proxy_battery.percentage() {
        // println!("battery percentage: {:.1}", perc);
        // }
        let _id = proxy_battery.match_signal(
            |h: battery::OrgFreedesktopDBusPropertiesPropertiesChanged,
             _: &Connection,
             _: &Message| {
                let perc: Option<&f64> = arg::prop_cast(&h.changed_properties, "Percentage");
                if let Some(perc) = perc {
                    println!("battery percentage: {:.1}", perc);
                }
                true
            },
        );

        let proxy_login = dbus_system.with_proxy(
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            time::Duration::from_millis(5000),
        );
        let _id = proxy_login.match_signal(
            |h: login1::OrgFreedesktopLogin1ManagerPrepareForSleep, _: &Connection, _: &Message| {
                println!("PrepareForSleep {:?}", h.start);
                if let Ok(mut d) = DEV_MANAGER.lock() {
                    d.set_ac_state_get();
                    if h.start {
                        d.light_off();
                    } else {
                        d.restore_light();

                        // The system just woke up. UPower can sometimes be slow to update its internal AC state
                        // and fire DBus signals. So we wait a few seconds and forcefully check again.
                        thread::spawn(|| {
                            thread::sleep(time::Duration::from_secs(3));
                            if let Ok(mut dev) = DEV_MANAGER.lock() {
                                println!("Delayed AC state check after wake");
                                dev.set_ac_state_get();
                            }
                        });
                    }
                }
                true
            },
        );
        // use login1::OrgFreedesktopLogin1ManagerPrepareForSleep;
        loop {
            if let Err(e) = dbus_system.process(time::Duration::from_millis(1000)) {
                eprintln!("Battery monitor D-Bus error: {}", e);
                thread::sleep(time::Duration::from_secs(1));
            }
        }
    })
}

/// Monitors signals and stops the daemon when receiving one
pub fn start_shutdown_task() -> JoinHandle<()> {
    thread::spawn(|| {
        let mut signals = Signals::new([SIGINT, SIGTERM]).unwrap();
        let _ = signals.forever().next();

        // If we reach this point, we have a signal and it is time to exit
        println!("Received signal, cleaning up");
        save_and_exit(0);
    })
}

/// Exit status meaning "restart me into the updated binary" - the service unit restarts on it.
const EXIT_UPDATED: i32 = 75;

/// Exits once the daemon binary is replaced by a reinstall or package update, so systemd starts
/// the new version (package managers can't restart a per-user service themselves). Without this,
/// the old daemon would keep answering a newer GUI whose socket protocol it doesn't match.
fn start_update_watch_task() -> JoinHandle<()> {
    thread::spawn(|| {
        let mut watch = service::UpdateWatch::default();
        loop {
            thread::sleep(time::Duration::from_secs(5));
            if watch.poll().is_some() {
                println!("Daemon binary was updated, restarting into the new version");
                save_and_exit(EXIT_UPDATED);
            }
        }
    })
}

/// Saves the effect layers and removes the socket, then exits with `code`.
fn save_and_exit(code: i32) -> ! {
    let json = match EFFECT_MANAGER.lock() {
        Ok(mut mgr) => mgr.save(),
        Err(e) => {
            eprintln!("Failed to lock effect manager for save: {}", e);
            serde_json::json!({"effects": []})
        }
    };
    if let Err(error) = config::Configuration::write_effects_save(json) {
        error!("Error writing config {}", error);
    }
    let _ = std::fs::remove_file(comms::socket_path());
    std::process::exit(code);
}

/// Far above any real request (the largest is well under 1 KiB) - just a bound on memory.
const MAX_REQUEST_BYTES: u64 = 64 * 1024;

fn handle_data(mut stream: UnixStream) {
    // Requests are served one at a time, so a client that connects and stalls (or streams
    // forever) must not be able to block every other client indefinitely.
    let timeout = Some(time::Duration::from_secs(2));
    let _ = stream.set_read_timeout(timeout);
    let _ = stream.set_write_timeout(timeout);

    let mut buffer = Vec::new();
    if let Err(error) = (&mut stream)
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut buffer)
    {
        eprintln!("Failed to read request from socket: {error}");
        return;
    }

    if buffer.is_empty() {
        eprintln!("Received empty request payload");
        return;
    }
    if buffer.len() as u64 > MAX_REQUEST_BYTES {
        eprintln!("Rejected oversized request ({} bytes)", buffer.len());
        return;
    }

    if let Some(cmd) = comms::read_from_socket_req(&buffer) {
        if let Some(s) = process_client_request(cmd) {
            if let Ok(x) = bincode::serialize(&s) {
                let result = stream.write_all(&x);

                if let Err(error) = result {
                    println!("Client disconnected with error: {error}");
                }
            }
        } else {
            eprintln!("No response for client request — closing connection");
        }
    } else {
        eprintln!("Failed to deserialize client request");
    }
}

pub fn process_client_request(cmd: comms::DaemonCommand) -> Option<comms::DaemonResponse> {
    // GPU commands don't need DEV_MANAGER, handle them first
    match &cmd {
        comms::DaemonCommand::GetGpuStatus => {
            let gpus = gpu::discover_gpus();
            let dgpu_rpm = gpu::get_dgpu_runtime_pm();
            let ec_available = gpu::envycontrol_available();
            let ec_mode = if ec_available {
                gpu::get_envycontrol_mode()
            } else {
                "unknown".to_string()
            };
            return Some(comms::DaemonResponse::GetGpuStatus {
                gpus,
                dgpu_runtime_pm: dgpu_rpm,
                envycontrol_mode: ec_mode,
                envycontrol_available: ec_available,
            });
        }
        comms::DaemonCommand::SetDgpuRuntimePM { enabled } => {
            return Some(comms::DaemonResponse::SetDgpuRuntimePM {
                result: gpu::set_dgpu_runtime_pm(*enabled),
            });
        }
        comms::DaemonCommand::SetGpuMode { mode } => {
            let (ok, msg) = gpu::set_envycontrol_mode(mode);
            return Some(comms::DaemonResponse::SetGpuMode {
                result: ok,
                message: msg,
            });
        }
        _ => {}
    }

    if let Ok(mut d) = DEV_MANAGER.lock() {
        match cmd {
            comms::DaemonCommand::SetPowerMode { ac, pwr, cpu, gpu } if ac < 2 => {
                Some(comms::DaemonResponse::SetPowerMode {
                    result: d.set_power_mode(ac, pwr, cpu, gpu),
                })
            }
            comms::DaemonCommand::SetFanSpeed { ac, rpm } if ac < 2 => {
                Some(comms::DaemonResponse::SetFanSpeed {
                    result: d.set_fan_rpm(ac, rpm),
                })
            }
            comms::DaemonCommand::SetLogoLedState { ac, logo_state } if ac < 2 => {
                Some(comms::DaemonResponse::SetLogoLedState {
                    result: d.set_logo_led_state(ac, logo_state),
                })
            }
            comms::DaemonCommand::SetBrightness { ac, val } if ac < 2 => {
                Some(comms::DaemonResponse::SetBrightness {
                    result: d.set_brightness(ac, val),
                })
            }
            comms::DaemonCommand::SetIdle { ac, val } if ac < 2 => {
                Some(comms::DaemonResponse::SetIdle {
                    result: d.change_idle(ac, val),
                })
            }
            comms::DaemonCommand::SetSync { sync } => Some(comms::DaemonResponse::SetSync {
                result: d.set_sync(sync),
            }),
            comms::DaemonCommand::GetBrightness { ac } if ac < 2 => {
                Some(comms::DaemonResponse::GetBrightness {
                    result: d.get_brightness(ac),
                })
            }
            comms::DaemonCommand::GetLogoLedState { ac } if ac < 2 => {
                Some(comms::DaemonResponse::GetLogoLedState {
                    logo_state: d.get_logo_led_state(ac),
                })
            }
            comms::DaemonCommand::GetKeyboardRGB { layer } => {
                if let Ok(mut mgr) = EFFECT_MANAGER.lock() {
                    Some(comms::DaemonResponse::GetKeyboardRGB {
                        layer,
                        rgbdata: mgr.get_map(layer),
                    })
                } else {
                    None
                }
            }
            comms::DaemonCommand::GetSync() => {
                Some(comms::DaemonResponse::GetSync { sync: d.get_sync() })
            }
            comms::DaemonCommand::GetFanSpeed { ac } if ac < 2 => {
                Some(comms::DaemonResponse::GetFanSpeed {
                    rpm: d.get_fan_rpm(ac),
                })
            }
            comms::DaemonCommand::GetPwrLevel { ac } if ac < 2 => {
                Some(comms::DaemonResponse::GetPwrLevel {
                    pwr: d.get_power_mode(ac),
                })
            }
            comms::DaemonCommand::GetCPUBoost { ac } if ac < 2 => {
                Some(comms::DaemonResponse::GetCPUBoost {
                    cpu: d.get_cpu_boost(ac),
                })
            }
            comms::DaemonCommand::GetGPUBoost { ac } if ac < 2 => {
                Some(comms::DaemonResponse::GetGPUBoost {
                    gpu: d.get_gpu_boost(ac),
                })
            }
            comms::DaemonCommand::SetEffect { name, params } => {
                stop_live_inputs();
                let mut res = false;
                let gui_idx = match name.as_str() {
                    "static" => 0u8,
                    "static_gradient" => 1,
                    "wave_gradient" => 2,
                    "breathing_single" => 3,
                    _ => 255,
                };
                // Persist GUI effect selection to config
                if gui_idx < 255 {
                    d.save_gui_effect(gui_idx, params.clone());
                }

                if d.device_has_feature("per_key_rgb") {
                    // Per-key RGB: push to EFFECT_MANAGER for animation loop
                    if let Ok(mut k) = EFFECT_MANAGER.lock() {
                        res = true;
                        let effect = match name.as_str() {
                            "static" => Some(kbd::effects::Static::new(params)),
                            "static_gradient" => Some(kbd::effects::StaticGradient::new(params)),
                            "wave_gradient" => Some(kbd::effects::WaveGradient::new(params)),
                            "breathing_single" => Some(kbd::effects::BreathSingle::new(params)),
                            _ => None,
                        };

                        if let Some(laptop) = d.get_device() {
                            if let Some(e) = effect {
                                k.pop_effect(laptop); // Remove old layer
                                k.push_effect(e, [true; kbd::TOTAL_KEYS]);
                            } else {
                                res = false
                            }
                        } else {
                            res = false;
                        }
                    }
                } else {
                    // No per-key RGB: map GUI effects to standard hardware effects
                    let (effect_id, hw_params) = match name.as_str() {
                        "static" => (device::RazerLaptop::STATIC, params),
                        "breathing_single" => (device::RazerLaptop::BREATHING, params),
                        "wave_gradient" => (device::RazerLaptop::WAVE, params),
                        "static_gradient" => (device::RazerLaptop::STATIC, params),
                        _ => (device::RazerLaptop::SPECTRUM, vec![]),
                    };
                    res = d.set_standard_effect(effect_id, hw_params);
                }
                Some(comms::DaemonResponse::SetEffect { result: res })
            }

            comms::DaemonCommand::SetStandardEffect { name, params } => {
                // TODO save standard effect may be struct ?
                stop_live_inputs();
                let mut res = false;
                // Computed before `d.get_device()` below borrows `d` mutably as `laptop` for the
                // rest of this block - `d.device_has_feature` needs an immutable borrow of `d`,
                // which would conflict with `laptop` still being alive when the "static" arm below
                // uses it.
                let has_per_key_rgb = d.device_has_feature("per_key_rgb");
                if let Some(laptop) = d.get_device() {
                    if let Ok(mut k) = EFFECT_MANAGER.lock() {
                        k.pop_effect(laptop); // Remove old layer
                        let _res = match name.as_str() {
                            "off" => d.set_standard_effect(device::RazerLaptop::OFF, params),
                            "wave" => d.set_standard_effect(device::RazerLaptop::WAVE, params),
                            "reactive" => {
                                d.set_standard_effect(device::RazerLaptop::REACTIVE, params)
                            }
                            "breathing" => {
                                // The native hardware Breathing effect is byte-verified-correct
                                // against OpenRazer's reference on this exact device but never
                                // produces any visible output (see kbd::effects::SoftBreathing's
                                // doc comment) - substitute a software per-key fade for devices
                                // that support it, leaving other hardware's native path (which
                                // may work fine there) untouched.
                                if has_per_key_rgb {
                                    k.push_effect(
                                        kbd::effects::SoftBreathing::new(params),
                                        [true; kbd::TOTAL_KEYS],
                                    );
                                    true
                                } else {
                                    d.set_standard_effect(device::RazerLaptop::BREATHING, params)
                                }
                            }
                            "spectrum" => {
                                d.set_standard_effect(device::RazerLaptop::SPECTRUM, params)
                            }
                            "static" => {
                                // Same signature as the native Breathing/Wheel bugs already fixed
                                // this session: the daemon-level HID trace shows a byte-perfect
                                // request and a clean RAZER_CMD_SUCCESSFUL ACK every time, but the
                                // keyboard never actually shows the new color and never
                                // self-corrects - the firmware's native STATIC mode-switch is
                                // unreliable on this unit. Bypass it with the per-key custom-frame
                                // fill (same mechanism as the GUI's "Fill All" paint button, which
                                // has been reliable all session) for devices that support it.
                                if has_per_key_rgb {
                                    let r = *params.first().unwrap_or(&0);
                                    let g = *params.get(1).unwrap_or(&0);
                                    let b = *params.get(2).unwrap_or(&0);
                                    k.fill_custom(r, g, b, laptop);
                                    true
                                } else {
                                    d.set_standard_effect(device::RazerLaptop::STATIC, params)
                                }
                            }
                            "starlight" => {
                                d.set_standard_effect(device::RazerLaptop::STARLIGHT, params)
                            }
                            _ => false,
                        };
                        res = _res;
                    }
                } else {
                    res = false;
                }
                Some(comms::DaemonResponse::SetStandardEffect { result: res })
            }
            comms::DaemonCommand::SetCustomKey { index, r, g, b } => {
                stop_live_inputs();
                let mut res = false;
                if let Some(laptop) = d.get_device()
                    && let Ok(mut k) = EFFECT_MANAGER.lock()
                {
                    res = k.set_custom_key(index as usize, r, g, b, laptop);
                }
                Some(comms::DaemonResponse::SetCustomKey { result: res })
            }
            comms::DaemonCommand::FillCustomFrame { r, g, b } => {
                stop_live_inputs();
                let mut res = false;
                if let Some(laptop) = d.get_device()
                    && let Ok(mut k) = EFFECT_MANAGER.lock()
                {
                    k.fill_custom(r, g, b, laptop);
                    res = true;
                }
                Some(comms::DaemonResponse::FillCustomFrame { result: res })
            }
            comms::DaemonCommand::RandomizeCustomFrame => {
                stop_live_inputs();
                let mut res = false;
                if let Some(laptop) = d.get_device()
                    && let Ok(mut k) = EFFECT_MANAGER.lock()
                {
                    k.randomize_custom(laptop);
                    res = true;
                }
                Some(comms::DaemonResponse::RandomizeCustomFrame { result: res })
            }
            comms::DaemonCommand::SetWheelEffect { direction, speed } => {
                // The hardware's native Wheel effect produced no visible output on this laptop
                // after exhaustive verification (see device.rs history) - this is now a real
                // software animation on the per-key custom-frame engine instead, using the same
                // wire direction/speed values the GUI already sends.
                stop_live_inputs();
                let mut res = false;
                if let Some(laptop) = d.get_device()
                    && let Ok(mut k) = EFFECT_MANAGER.lock()
                {
                    k.pop_effect(laptop); // Remove old layer
                    k.push_effect(
                        kbd::effects::Wheel::new(vec![direction, speed]),
                        [true; kbd::TOTAL_KEYS],
                    );
                    res = true;
                }
                Some(comms::DaemonResponse::SetWheelEffect { result: res })
            }
            comms::DaemonCommand::SetSoundBarEffect {
                color_mode,
                r,
                g,
                b,
                sensitivity,
                decay,
                brightness,
            } => {
                stop_key_capture();
                let mut res = false;
                if let Ok(mut cfg) = AUDIO_CONFIG.lock() {
                    cfg.sensitivity = sensitivity as f32 / 100.0;
                    cfg.decay = decay as f32 / 100.0;
                }
                if let Some(laptop) = d.get_device()
                    && let Ok(mut k) = EFFECT_MANAGER.lock()
                    && let Ok(mut a) = AUDIO.lock()
                {
                    if a.is_none() {
                        *a = audio::AudioCapture::start(AUDIO_CONFIG.clone());
                    }
                    if let Some(capture) = a.as_ref() {
                        k.pop_effect(laptop); // Remove old layer
                        k.push_effect(
                            kbd::effects::SoundBar::new_live(
                                vec![color_mode, r, g, b, brightness],
                                capture.levels.clone(),
                            ),
                            [true; kbd::TOTAL_KEYS],
                        );
                        res = true;
                    }
                }
                Some(comms::DaemonResponse::SetSoundBarEffect { result: res })
            }
            comms::DaemonCommand::SetStarsEffect { speed } => {
                stop_live_inputs();
                let mut res = false;
                if let Some(laptop) = d.get_device()
                    && let Ok(mut k) = EFFECT_MANAGER.lock()
                {
                    k.pop_effect(laptop); // Remove old layer
                    k.push_effect(
                        kbd::effects::Stars::new(vec![speed]),
                        [true; kbd::TOTAL_KEYS],
                    );
                    res = true;
                }
                Some(comms::DaemonResponse::SetStarsEffect { result: res })
            }
            comms::DaemonCommand::SetRippleEffect {
                color_mode,
                r,
                g,
                b,
                speed,
            } => {
                stop_audio_capture();
                let mut res = false;
                if let Some(laptop) = d.get_device()
                    && let Ok(mut k) = EFFECT_MANAGER.lock()
                    && let Ok(mut keys) = KEYS.lock()
                {
                    if keys.is_none() {
                        *keys = keys::KeyCapture::start(laptop.get_pid());
                    }
                    if let Some(capture) = keys.as_ref() {
                        k.pop_effect(laptop); // Remove old layer
                        k.push_effect(
                            kbd::effects::Ripple::new_live(
                                vec![color_mode, r, g, b, speed],
                                capture.presses.clone(),
                            ),
                            [true; kbd::TOTAL_KEYS],
                        );
                        res = true;
                    }
                }
                Some(comms::DaemonResponse::SetRippleEffect { result: res })
            }
            comms::DaemonCommand::SetBatteryHealthOptimizer { is_on, threshold } => {
                Some(comms::DaemonResponse::SetBatteryHealthOptimizer {
                    result: d.set_bho_handler(is_on, threshold),
                })
            }
            comms::DaemonCommand::GetBatteryHealthOptimizer() => {
                // Don't abort the whole batched status response when BHO is unsupported.
                // Return a harmless default so the client still gets every other field.
                let (is_on, threshold) = d.get_bho_handler().unwrap_or((false, 0));
                Some(comms::DaemonResponse::GetBatteryHealthOptimizer { is_on, threshold })
            }
            comms::DaemonCommand::GetActualFanRpm => Some(comms::DaemonResponse::GetActualFanRpm {
                rpm: d.get_actual_fan_rpm(),
            }),
            comms::DaemonCommand::GetDeviceName => {
                let name = match &d.device {
                    Some(device) => device.get_name(),
                    None => "Unknown Device".into(),
                };
                Some(comms::DaemonResponse::GetDeviceName { name })
            }
            comms::DaemonCommand::GetStandardEffect => {
                let (effect, params) = d.get_standard_effect();
                Some(comms::DaemonResponse::GetStandardEffect { effect, params })
            }
            // Reject commands with invalid ac index (>= 2)
            _ => {
                eprintln!("Rejected command with invalid ac index: {:?}", cmd);
                None
            }
        }
    } else {
        None
    }
}
