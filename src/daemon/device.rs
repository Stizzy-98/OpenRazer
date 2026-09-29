use crate::battery;
use crate::config;
use crate::dbus_mutter_idlemonitor;
use dbus::blocking::Connection;
use hidapi::HidApi;
use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;
use service::{SupportedDevice, device_file_path};
use std::ffi::CString;
use std::{fs, io, thread, time};

const RAZER_VENDOR_ID: u16 = 0x1532;

#[derive(Serialize, Deserialize, Debug)]
pub struct RazerPacket {
    report: u8,
    status: u8,
    id: u8,
    remaining_packets: u16,
    protocol_type: u8,
    data_size: u8,
    command_class: u8,
    command_id: u8,
    #[serde(with = "BigArray")]
    args: [u8; 80],
    crc: u8,
    reserved: u8,
}

impl RazerPacket {
    // Command status
    const RAZER_CMD_NEW: u8 = 0x00;
    // const RAZER_CMD_BUSY:u8 = 0x01;
    const RAZER_CMD_SUCCESSFUL: u8 = 0x02;
    // const RAZER_CMD_FAILURE:u8 = 0x03;
    // const RAZER_CMD_TIMEOUT:u8 =0x04;
    const RAZER_CMD_NOT_SUPPORTED: u8 = 0x05;

    fn new(command_class: u8, command_id: u8, data_size: u8) -> RazerPacket {
        RazerPacket {
            report: 0x00,
            status: RazerPacket::RAZER_CMD_NEW,
            id: 0x1F,
            remaining_packets: 0x0000,
            protocol_type: 0x00,
            data_size,
            command_class,
            command_id,
            args: [0x00; 80],
            crc: 0x00,
            reserved: 0x00,
        }
    }

    /// OpenRazer's `razer_calculate_crc` XORs `razer_report` bytes [2, 88) - remaining_packets
    /// through the last argument. That struct has no leading report-id byte and ours does, so the
    /// same range is [3, 89) here, and the CRC byte itself sits at index 89.
    fn calc_crc(&mut self) -> Vec<u8> {
        let mut buf: Vec<u8> = bincode::serialize(self).unwrap();
        let crc = buf[3..89].iter().fold(0u8, |acc, byte| acc ^ byte);
        self.crc = crc;
        buf[89] = crc;
        buf
    }
}

pub struct DeviceManager {
    pub device: Option<RazerLaptop>,
    supported_devices: Vec<SupportedDevice>,
    pub config: Option<config::Configuration>,
    pub idle_id: u32,
    pub active_id: u32,
    add_active: bool,
    pub change_idle: bool,
}

impl DeviceManager {
    /// Read the USB interface number for a /dev/hidrawX node from sysfs.
    fn hidraw_iface_number(hidraw_name: &str) -> Option<i32> {
        let iface_path = format!(
            "/sys/class/hidraw/{}/device/../bInterfaceNumber",
            hidraw_name
        );
        let raw = fs::read_to_string(iface_path).ok()?;
        i32::from_str_radix(raw.trim(), 16).ok()
    }

    fn read_hex_u16(path: &std::path::Path) -> Option<u16> {
        let raw = fs::read_to_string(path).ok()?;
        let trimmed = raw.trim();
        u16::from_str_radix(trimmed, 16).ok()
    }

    /// Resolve VID/PID for a /dev/hidrawX node via /sys, walking up parents
    /// until we find idVendor/idProduct.
    fn hidraw_vid_pid(hidraw_name: &str) -> Option<(u16, u16)> {
        let mut current =
            fs::canonicalize(format!("/sys/class/hidraw/{}/device", hidraw_name)).ok()?;

        for _ in 0..6 {
            let vid_path = current.join("idVendor");
            let pid_path = current.join("idProduct");
            if vid_path.exists() && pid_path.exists() {
                let vid = Self::read_hex_u16(&vid_path)?;
                let pid = Self::read_hex_u16(&pid_path)?;
                return Some((vid, pid));
            }
            if !current.pop() {
                break;
            }
        }
        None
    }

    pub fn new() -> DeviceManager {
        DeviceManager {
            device: None,
            supported_devices: vec![],
            config: None,
            idle_id: 0,
            active_id: 0,
            add_active: false,
            change_idle: false,
        }
    }

    pub fn add_idle_watch(
        &mut self,
        proxy_idle: &dyn dbus_mutter_idlemonitor::OrgGnomeMutterIdleMonitor,
    ) {
        if self.change_idle {
            let mut timeout: u64 = 0;
            let mut state: usize = 0;
            if let Some(laptop) = self.get_device() {
                state = laptop.get_ac_state();
            }
            if let Some(config) = self.get_config() {
                timeout = config.power[state].idle as u64 * 60 * 1000; // idle is in minutes timeout is in miliseconds
            }
            if timeout != 0 {
                if self.idle_id != 0 {
                    self.remove_watch(proxy_idle);
                }
                if let Ok(id) = proxy_idle.add_idle_watch(timeout) {
                    println!("idle handler {:?}", id);
                    self.idle_id = id;
                }
            } else {
                if self.idle_id != 0 {
                    self.remove_watch(proxy_idle);
                }
            }
            self.change_idle = false;
        }
    }

    pub fn set_sync(&mut self, sync: bool) -> bool {
        let mut ac: usize = 0;
        if let Some(laptop) = self.get_device() {
            ac = laptop.ac_state as usize;
        }
        let other = (ac + 1) & 0x01;
        if let Some(config) = self.get_config() {
            config.sync = sync;
            config.power[other].brightness = config.power[ac].brightness;
            config.power[other].logo_state = config.power[ac].logo_state;
            config.power[other].screensaver = config.power[ac].screensaver;
            config.power[other].idle = config.power[ac].idle;
            if let Err(e) = config.write_to_file() {
                eprintln!("Error write config {:?}", e);
            }
        }

        true
    }

    pub fn get_sync(&mut self) -> bool {
        if let Some(config) = self.get_config() {
            return config.sync;
        }

        false
    }

    fn remove_watch(
        &mut self,
        proxy_idle: &dyn dbus_mutter_idlemonitor::OrgGnomeMutterIdleMonitor,
    ) {
        if proxy_idle.remove_watch(self.idle_id).is_ok() {
            println!("remove idle handler");
        }
    }

    pub fn add_active_watch(
        &mut self,
        proxy_idle: &dyn dbus_mutter_idlemonitor::OrgGnomeMutterIdleMonitor,
    ) {
        if self.add_active
            && let Ok(id) = proxy_idle.add_user_active_watch()
        {
            println!("active handler {:?}", id);
            self.active_id = id;
        }
    }

    pub fn read_laptops_file() -> io::Result<DeviceManager> {
        let path = device_file_path();
        let str: Vec<u8> = fs::read(&path)?;
        let mut res: DeviceManager = DeviceManager::new();
        res.supported_devices = serde_json::from_slice(str.as_slice())?;
        println!("supported devices found: {:?}", res.supported_devices.len());
        match config::Configuration::read_from_config() {
            Ok(c) => res.config = Some(c),
            Err(_) => res.config = Some(config::Configuration::new()),
        }

        Ok(res)
    }

    fn get_ac_config(&mut self, ac: usize) -> Option<config::PowerConfig> {
        if let Some(c) = self.get_config() {
            return Some(c.power[ac]);
        }

        None
    }

    pub fn light_off(&mut self) {
        if self.idle_id != 0 {
            self.add_active = true;
        }
        if let Some(laptop) = self.get_device() {
            laptop.set_screensaver(true);
            laptop.set_brightness(0);
            laptop.set_logo_led_state(0);
        }
    }

    pub fn restore_light(&mut self) {
        self.add_active = false;
        let mut brightness = 0;
        let mut logo_state = 0;
        let mut ac: usize = 0;
        if let Some(laptop) = self.get_device() {
            ac = laptop.get_ac_state();
        }
        if let Some(config) = self.get_ac_config(ac) {
            brightness = config.brightness;
            logo_state = config.logo_state;
        }
        if let Some(laptop) = self.get_device() {
            laptop.set_screensaver(false);
            laptop.set_brightness(brightness);
            laptop.set_logo_led_state(logo_state);
        }
    }

    /// Columns in the detected model's per-key lighting grid.
    pub fn matrix_cols(&self) -> usize {
        let Some(pid) = self.device.as_ref().map(|d| d.pid) else {
            return 16;
        };
        self.supported_devices
            .iter()
            .find(|d| u16::from_str_radix(&d.pid, 16) == Ok(pid))
            .map_or(16, |d| d.matrix_cols())
    }

    /// Check whether the current device declares a given feature.
    pub fn device_has_feature(&self, feature: &str) -> bool {
        self.device
            .as_ref()
            .is_some_and(|d| d.features.contains(&feature.to_string()))
    }

    pub fn restore_standard_effect(&mut self) {
        let mut effect = 0;
        let mut params: Vec<u8> = vec![];
        if let Some(config) = self.get_config() {
            effect = config.standard_effect;
            params = config.standard_effect_params.clone();
        }
        if let Some(laptop) = self.get_device() {
            laptop.set_standard_effect(effect, params);
        }
    }

    pub fn change_idle(&mut self, ac: usize, timeout: u32) -> bool {
        // let mut arm: bool = false;
        if let Some(config) = self.get_config()
            && config.power[ac].idle != timeout
        {
            config.power[ac].idle = timeout;
            if config.sync {
                let other = (ac + 1) & 0x01;
                config.power[other].idle = timeout;
            }
            if let Err(e) = config.write_to_file() {
                eprintln!("Error write config {:?}", e);
            }
            // arm = true;
            self.change_idle = true;
        }

        true
    }

    pub fn set_power_mode(&mut self, ac: usize, pwr: u8, cpu: u8, gpu: u8) -> bool {
        // Reject out-of-range values before they are persisted: the config is
        // replayed to the EC on every start, AC switch and resume.
        if pwr > POWER_MODE_CUSTOM || cpu > 3 || gpu > 2 {
            eprintln!("Rejected power mode {pwr} (cpu {cpu}, gpu {gpu}): out of range");
            return false;
        }
        let mut res: bool = false;
        if let Some(config) = self.get_config() {
            config.power[ac].power_mode = pwr;
            config.power[ac].cpu_boost = cpu;
            config.power[ac].gpu_boost = gpu;
            if let Err(e) = config.write_to_file() {
                eprintln!("Error write config {:?}", e);
            }
        }
        if let Some(laptop) = self.get_device() {
            let state = laptop.get_ac_state();
            if state != ac {
                res = true;
            } else {
                res = laptop.set_power_mode(pwr, cpu, gpu);
            }
        }

        res
    }

    pub fn get_standard_effect(&mut self) -> (u8, Vec<u8>) {
        // Must read the same fields `set_standard_effect`/`restore_standard_effect` use
        // (config.standard_effect*), not config.gui_effect* - that field belongs to the
        // separate, older SetEffect/`razer-cli effect ...` path and is never touched by
        // SetStandardEffect, so reading it here returned stale/unrelated data.
        if let Some(config) = self.get_config() {
            return (
                config.standard_effect,
                config.standard_effect_params.clone(),
            );
        }
        (0, vec![])
    }

    pub fn save_gui_effect(&mut self, effect_idx: u8, params: Vec<u8>) {
        if let Some(config) = self.get_config() {
            config.gui_effect = effect_idx;
            config.gui_effect_params = params;
            if let Err(e) = config.write_to_file() {
                eprintln!("Error write config {:?}", e);
            }
        }
    }

    pub fn set_standard_effect(&mut self, effect_id: u8, params: Vec<u8>) -> bool {
        if let Some(config) = self.get_config() {
            config.standard_effect = effect_id;
            config.standard_effect_params = params.clone();
            if let Err(e) = config.write_to_file() {
                eprintln!("Error write config {:?}", e);
            }
        }
        if let Some(laptop) = self.get_device() {
            laptop.set_standard_effect(effect_id, params);
        }

        true
    }

    pub fn set_fan_rpm(&mut self, ac: usize, rpm: i32) -> bool {
        let mut res: bool = false;
        if let Some(config) = self.get_config() {
            config.power[ac].fan_rpm = rpm;
            if let Err(e) = config.write_to_file() {
                eprintln!("Error write config {:?}", e);
            }
        }

        if let Some(laptop) = self.get_device() {
            let state = laptop.get_ac_state();
            if state != ac {
                res = true;
            } else {
                res = laptop.set_fan_rpm(rpm as u16);
            }
        }

        res
    }

    pub fn set_logo_led_state(&mut self, ac: usize, logo_state: u8) -> bool {
        let mut res: bool = false;
        let mut is_synced = false;

        if let Some(config) = self.get_config() {
            is_synced = config.sync;
            config.power[ac].logo_state = logo_state;
            if config.sync {
                let other = (ac + 1) & 0x01;
                config.power[other].logo_state = logo_state;
            }
            if let Err(e) = config.write_to_file() {
                eprintln!("Error write config {:?}", e);
            }
        }

        if let Some(laptop) = self.get_device() {
            let state = laptop.get_ac_state();

            if state != ac && !is_synced {
                res = true;
            } else {
                res = laptop.set_logo_led_state(logo_state);
            }
        }

        res
    }

    pub fn get_logo_led_state(&mut self, ac: usize) -> u8 {
        // if let Some(laptop) = self.get_device() {
        // if laptop.ac_state as usize == ac {
        // return laptop.get_logo_led_state();
        // }
        // }

        if let Some(config) = self.get_ac_config(ac) {
            return config.logo_state;
        }

        0
    }

    pub fn set_brightness(&mut self, ac: usize, brightness: u8) -> bool {
        let mut res: bool = false;
        let clamped = if brightness > 100 {
            100u16
        } else {
            brightness as u16
        };
        let _val = clamped * 255 / 100;
        let mut is_synced = false;

        if let Some(config) = self.get_config() {
            is_synced = config.sync;
            config.power[ac].brightness = _val as u8;
            if config.sync {
                let other = (ac + 1) & 0x01;
                config.power[other].brightness = _val as u8;
            }
            if let Err(e) = config.write_to_file() {
                eprintln!("Error write config {:?}", e);
            }
        }

        if let Some(laptop) = self.get_device() {
            let state = laptop.get_ac_state();
            // If sync is enabled, the new brightness applies to both states, so update hardware regardless
            if state != ac && !is_synced {
                res = true;
            } else {
                res = laptop.set_brightness(_val as u8);
            }
        }

        res
    }

    pub fn get_brightness(&mut self, ac: usize) -> u8 {
        // For the active power state, trust the keyboard over the saved value: the Fn brightness
        // keys change it without going through the daemon. Not while the lights are off for
        // idle/screensaver, when the keyboard reads 0 on purpose.
        let live = self
            .get_device()
            .filter(|l| l.get_ac_state() == ac && !l.is_screensaver())
            .and_then(|l| l.read_brightness());
        if let Some(value) = live
            && let Some(config) = self.get_config()
            && config.power[ac].brightness != value
        {
            config.power[ac].brightness = value;
            if config.sync {
                config.power[(ac + 1) & 0x01].brightness = value;
            }
            if let Err(e) = config.write_to_file() {
                eprintln!("Error write config {:?}", e);
            }
        }
        if let Some(config) = self.get_ac_config(ac) {
            let val = config.brightness as u32;
            let mut perc = val * 100 * 100 / 255;
            perc += 50;
            perc /= 100;
            return perc as u8;
        }

        0
    }

    pub fn get_actual_fan_rpm(&mut self) -> i32 {
        if let Some(laptop) = self.get_device() {
            return laptop.read_fan_rpm_from_ec() as i32;
        }
        0
    }

    pub fn get_fan_rpm(&mut self, ac: usize) -> i32 {
        let live_fan_setting = {
            if let Some(laptop) = self.get_device() {
                let state = laptop.get_ac_state();
                if state == ac {
                    laptop.read_fan_setting().map(|rpm| rpm as i32)
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some(rpm) = live_fan_setting {
            return rpm;
        }

        if let Some(config) = self.get_ac_config(ac) {
            return config.fan_rpm;
        }

        0
    }

    pub fn get_power_mode(&mut self, ac: usize) -> u8 {
        if let Some(config) = self.get_ac_config(ac) {
            return config.power_mode;
        }

        0
    }

    pub fn get_cpu_boost(&mut self, ac: usize) -> u8 {
        if let Some(config) = self.get_ac_config(ac) {
            return config.cpu_boost;
        }

        0
    }

    pub fn get_gpu_boost(&mut self, ac: usize) -> u8 {
        if let Some(config) = self.get_ac_config(ac) {
            return config.gpu_boost;
        }

        0
    }

    pub fn set_ac_state(&mut self, ac: bool) {
        if let Some(laptop) = self.get_device() {
            laptop.set_ac_state(ac);
        }
        self.change_idle = true;
        let config: Option<config::PowerConfig> = self.get_ac_config(ac as usize);
        if let Some(config) = config
            && let Some(laptop) = self.get_device()
        {
            laptop.set_config(config);
        }
    }

    pub fn set_ac_state_get(&mut self) {
        let dbus_system = match Connection::new_system() {
            Ok(conn) => conn,
            Err(e) => {
                eprintln!("Failed to connect to D-Bus system bus: {}", e);
                return;
            }
        };
        let proxy_ac = dbus_system.with_proxy(
            "org.freedesktop.UPower",
            "/org/freedesktop/UPower/devices/line_power_AC0",
            time::Duration::from_millis(5000),
        );
        use battery::OrgFreedesktopUPowerDevice;
        if let Ok(online) = proxy_ac.online() {
            if let Some(laptop) = self.get_device() {
                laptop.set_ac_state(online);
            }
            self.change_idle = true;
            let config: Option<config::PowerConfig> = self.get_ac_config(online as usize);
            if let Some(config) = config
                && let Some(laptop) = self.get_device()
            {
                laptop.set_config(config);
            }
        }
    }

    pub fn get_device(&mut self) -> Option<&mut RazerLaptop> {
        self.device.as_mut()
    }

    pub fn set_bho_handler(&mut self, is_on: bool, threshold: u8) -> bool {
        // This byte goes straight to the battery controller (and values >= 128 would collide with
        // the on/off flag bit), so enforce the same range the UIs offer.
        if !service::valid_bho_threshold(threshold) {
            return false;
        }
        let result = self
            .get_device()
            .is_some_and(|laptop| laptop.set_bho(is_on, threshold));
        if result && let Some(config) = self.get_config() {
            config.bho_on = is_on;
            config.bho_threshold = threshold;
            if let Err(e) = config.write_to_file() {
                eprintln!("Error write config {:?}", e);
            }
        }
        result
    }

    pub fn get_bho_handler(&mut self) -> Option<(bool, u8)> {
        // Check if device supports BHO
        let has_bho = self
            .get_device()
            .is_some_and(|laptop| laptop.have_feature("bho".to_string()));
        if !has_bho {
            return None;
        }
        if let Some(config) = self.get_config() {
            return Some((config.bho_on, config.bho_threshold));
        }
        None
    }

    pub fn restore_bho(&mut self) {
        let (bho_on, bho_threshold) = {
            match self.get_config() {
                Some(config) => (config.bho_on, config.bho_threshold),
                None => return,
            }
        };
        if bho_on && let Some(laptop) = self.get_device() {
            laptop.set_bho(bho_on, bho_threshold);
        }
    }

    fn get_config(&mut self) -> Option<&mut config::Configuration> {
        self.config.as_mut()
    }

    // pub fn set_device(&mut self, device: RazerLaptop) {
    // self.device = Some(device);
    // }

    pub fn find_supported_device(&mut self, vid: u16, pid: u16) -> Option<&SupportedDevice> {
        for device in &self.supported_devices {
            let svid = u16::from_str_radix(&device.vid, 16).ok()?;
            let spid = u16::from_str_radix(&device.pid, 16).ok()?;

            if svid == vid && spid == pid {
                return Some(device);
            }
        }

        None
    }

    pub fn discover_devices(&mut self) {
        // Check if socket is OK
        match HidApi::new() {
            Ok(api) => {
                // Primary path: interface 0 via hidapi.
                // hidapi's linux-native (hidraw) backend returns -1 for
                // interface_number(), so resolve the real USB interface
                // number from sysfs when the value is unavailable.
                for device in api
                    .device_list()
                    .filter(|d| d.vendor_id() == RAZER_VENDOR_ID)
                {
                    let iface = if device.interface_number() >= 0 {
                        device.interface_number()
                    } else {
                        // Derive interface from sysfs via the device path
                        let path_str = device.path().to_str().unwrap_or_default();
                        let hidraw_name = path_str.rsplit('/').next().unwrap_or("");
                        Self::hidraw_iface_number(hidraw_name).unwrap_or(-1)
                    };
                    if iface != 0 {
                        continue;
                    }

                    if let Some(supported_device) =
                        self.find_supported_device(device.vendor_id(), device.product_id())
                    {
                        match api.open_path(device.path()) {
                            Ok(dev) => {
                                self.device = Some(RazerLaptop::new(
                                    supported_device.name.clone(),
                                    supported_device.features.clone(),
                                    supported_device.fan.clone(),
                                    device.product_id(),
                                    dev,
                                ));
                                return;
                            }
                            Err(e) => {
                                eprintln!(
                                    "Failed to open supported device on iface 0 ({:04x}:{:04x}): {}",
                                    device.vendor_id(),
                                    device.product_id(),
                                    e
                                );
                            }
                        }
                    }
                }

                // Fallback #1: direct /dev/hidrawX probing based on /sys VID/PID.
                // Collect candidates and sort by USB interface number so we
                // prefer interface 0 (the one that accepts feature reports).
                let mut candidates: Vec<(String, u16, u16, i32)> = Vec::new();
                if let Ok(entries) = fs::read_dir("/dev") {
                    for entry in entries.flatten() {
                        let name = match entry.file_name().into_string() {
                            Ok(n) => n,
                            Err(_) => continue,
                        };
                        if !name.starts_with("hidraw") {
                            continue;
                        }

                        let Some((vid, pid)) = Self::hidraw_vid_pid(&name) else {
                            continue;
                        };

                        if vid != RAZER_VENDOR_ID {
                            continue;
                        }

                        let iface = Self::hidraw_iface_number(&name).unwrap_or(999);
                        eprintln!(
                            "hidraw fallback candidate: /dev/{} vid={:04x} pid={:04x} iface={}",
                            name, vid, pid, iface
                        );
                        candidates.push((name, vid, pid, iface));
                    }
                }
                candidates.sort_by_key(|c| c.3); // prefer lowest interface number

                for (name, vid, pid, iface) in candidates {
                    if let Some(supported_device) = self.find_supported_device(vid, pid) {
                        let path = format!("/dev/{}", name);
                        let c_path = match CString::new(path.clone()) {
                            Ok(p) => p,
                            Err(_) => continue,
                        };
                        eprintln!(
                            "Trying hidraw fallback open for {} ({:04x}:{:04x}) on {} (iface {})",
                            supported_device.name, vid, pid, path, iface,
                        );
                        match api.open_path(c_path.as_c_str()) {
                            Ok(dev) => {
                                self.device = Some(RazerLaptop::new(
                                    supported_device.name.clone(),
                                    supported_device.features.clone(),
                                    supported_device.fan.clone(),
                                    pid,
                                    dev,
                                ));
                                return;
                            }
                            Err(e) => {
                                eprintln!(
                                    "hidraw fallback open failed for {} ({:04x}:{:04x}) on {}: {}",
                                    supported_device.name, vid, pid, path, e
                                );
                            }
                        }
                    }
                }

                eprintln!("No supported Razer HID device could be opened");
            }
            Err(e) => {
                eprintln!("Error: {}", e);
            }
        }
    }
}

pub struct RazerLaptop {
    name: String,
    pub(crate) features: Vec<String>,
    fan: Vec<u16>,
    pid: u16,
    device: hidapi::HidDevice,
    power: u8,    // need for fan
    fan_rpm: u8,  // need for power
    ac_state: u8, // index config array
    screensaver: bool,
}
//
impl RazerLaptop {
    // LED STORAGE Options
    const NOSTORE: u8 = 0x00;
    const VARSTORE: u8 = 0x01;
    // LED definitions
    const LOGO_LED: u8 = 0x04;
    const BACKLIGHT_LED: u8 = 0x05;
    // effects
    pub const OFF: u8 = 0x00;
    pub const WAVE: u8 = 0x01;
    pub const REACTIVE: u8 = 0x02; // Afterglo
    pub const BREATHING: u8 = 0x03;
    pub const SPECTRUM: u8 = 0x04;
    pub const CUSTOMFRAME: u8 = 0x05;
    pub const STATIC: u8 = 0x06;
    pub const STARLIGHT: u8 = 0x19;

    pub fn new(
        name: String,
        features: Vec<String>,
        fan: Vec<u16>,
        pid: u16,
        device: hidapi::HidDevice,
    ) -> RazerLaptop {
        RazerLaptop {
            name,
            features,
            fan,
            pid,
            device,
            power: 0,
            fan_rpm: 0,
            ac_state: 0,
            screensaver: false,
        }
    }

    pub fn set_screensaver(&mut self, active: bool) {
        self.screensaver = active;
    }

    pub fn is_screensaver(&self) -> bool {
        self.screensaver
    }

    pub fn set_config(&mut self, config: config::PowerConfig) -> bool {
        let mut ret: bool = false;

        if !self.screensaver {
            ret |= self.set_brightness(config.brightness);
            ret |= self.set_logo_led_state(config.logo_state);
        } else {
            ret |= self.set_brightness(0);
            ret |= self.set_logo_led_state(0);
        }
        ret |= self.set_power_mode(config.power_mode, config.cpu_boost, config.gpu_boost);
        ret |= self.set_fan_rpm(config.fan_rpm as u16);

        ret
    }

    pub fn set_ac_state(&mut self, online: bool) -> usize {
        if online {
            self.ac_state = 1;
        } else {
            self.ac_state = 0;
        }

        self.ac_state as usize
    }

    pub fn get_ac_state(&self) -> usize {
        self.ac_state as usize
    }

    pub fn get_name(&self) -> String {
        self.name.clone()
    }

    pub fn get_pid(&self) -> u16 {
        self.pid
    }

    pub fn have_feature(&mut self, fch: String) -> bool {
        self.features.contains(&fch)
    }

    fn clamp_fan(&mut self, rpm: u16) -> u8 {
        if rpm > self.fan[1] {
            return (self.fan[1] / 100) as u8;
        }
        if rpm < self.fan[0] {
            return (self.fan[0] / 100) as u8;
        }

        (rpm / 100) as u8
    }

    fn clamp_u8(&mut self, value: u8, min: u8, max: u8) -> u8 {
        if value > max {
            return max;
        }
        if value < min {
            return min;
        }

        value
    }

    /// The report's `data_size` field is a fixed constant *per effect type* in the real Chroma
    /// protocol (confirmed against OpenRazer's razerchromacommon.c reference implementation -
    /// e.g. Breathing always declares 0x08, Starlight always 0x01, regardless of how many of
    /// those single/dual/random modes' bytes are actually populated), not a byte count to
    /// compute from `params.len()`. Sending a wrong/oversized value here was silently tolerated
    /// by firmware for some effects but made Breathing a no-op entirely on real hardware.
    fn standard_effect_data_size(effect_id: u8) -> u8 {
        match effect_id {
            RazerLaptop::OFF => 0x01,
            RazerLaptop::WAVE => 0x02,
            RazerLaptop::REACTIVE => 0x05,
            RazerLaptop::BREATHING => 0x08,
            RazerLaptop::SPECTRUM => 0x01,
            RazerLaptop::CUSTOMFRAME => 0x02,
            RazerLaptop::STATIC => 0x04,
            RazerLaptop::STARLIGHT => 0x01,
            _ => 80,
        }
    }

    /// OpenRazer's razerkbd_driver.c sends every lighting command (standard effects, custom
    /// frames, brightness) to every Blade laptop and the Razer Book with transaction id 0xFF,
    /// not the 0x1F `RazerPacket::new` defaults to.
    const LIGHTING_TRANSACTION_ID: u8 = 0xFF;
    /// ...except custom-frame rows on the Blade Late 2016, which take 0x3F.
    const BLADE_LATE_2016_PID: u16 = 0x0224;

    pub fn set_standard_effect(&mut self, effect_id: u8, params: Vec<u8>) -> bool {
        let mut report: RazerPacket =
            RazerPacket::new(0x03, 0x0a, Self::standard_effect_data_size(effect_id));
        report.id = Self::LIGHTING_TRANSACTION_ID;
        report.args[0] = effect_id; // effect id
        if !params.is_empty() {
            let len = params.len().min(79); // args[0] is effect_id, so max 79 param bytes
            report.args[1..(len + 1)].copy_from_slice(&params[..len]);
        }
        if self.send_report(report).is_some() {
            return true;
        }

        false
    }

    /// Sends one row of the per-key frame: `data` is RGB for every column of the model's grid.
    pub fn set_custom_frame_data(&mut self, row: u8, data: Vec<u8>) {
        let cols = data.len() / 3;
        // args[4..] holds the RGB bytes and there are 80 args in total.
        if cols == 0 || !data.len().is_multiple_of(3) || data.len() > 76 {
            return;
        }
        // Layout per OpenRazer's razer_chroma_standard_matrix_set_custom_frame: header
        // [frame_id, row, start_col, stop_col], RGB from args[4], data_size fixed at 0x46.
        let mut report: RazerPacket = RazerPacket::new(0x03, 0x0b, 0x46);
        report.id = if self.pid == Self::BLADE_LATE_2016_PID {
            0x3F
        } else {
            Self::LIGHTING_TRANSACTION_ID
        };
        report.args[0] = 0xff;
        report.args[1] = row;
        report.args[2] = 0x00; // start col
        report.args[3] = (cols - 1) as u8; // stop col, inclusive
        report.args[4..(data.len() + 4)].copy_from_slice(&data[..]);
        self.send_report(report);
    }

    pub fn set_custom_frame(&mut self) -> bool {
        let mut report: RazerPacket = RazerPacket::new(0x03, 0x0a, 0x02);
        report.id = Self::LIGHTING_TRANSACTION_ID;
        report.args[0] = RazerLaptop::CUSTOMFRAME; // effect id
        report.args[1] = RazerLaptop::NOSTORE;
        if self.send_report(report).is_some() {
            return true;
        }

        false
    }

    // The hardware's native extended-matrix-effect Wheel (command_class 0x0F/0x02, effect id
    // 0x0A) was tried here directly against real hardware across every direction/speed
    // combination - always accepted as RAZER_CMD_SUCCESSFUL, never any visible output. OpenRazer's
    // own device table only lists this effect for BlackWidow V4-family keyboards, not this
    // laptop, and there's no further evidence to chase without genuinely new hardware access
    // (e.g. a real USB capture from a keyboard that's confirmed to display it). Replaced with a
    // real software animation - see kbd::effects::Wheel, driven by daemon.rs's SetWheelEffect.

    pub fn get_power_mode(&mut self, zone: u8) -> u8 {
        if let Some((mode_byte, _manual_flag)) = self.read_zone_fan_state(zone) {
            return mode_byte;
        }
        0
    }

    fn read_zone_fan_state(&mut self, zone: u8) -> Option<(u8, u8)> {
        let mut report: RazerPacket = RazerPacket::new(0x0d, 0x82, 0x04);
        report.args[0] = 0x00;
        report.args[1] = zone;
        report.args[2] = 0x00;
        report.args[3] = 0x00;
        self.send_report(report)
            .map(|response| (response.args[2], response.args[3]))
    }

    fn set_zone_fan_state(&mut self, zone: u8, mode_byte: u8, manual_flag: u8) -> bool {
        let mut report: RazerPacket = RazerPacket::new(0x0d, 0x02, 0x04);
        report.args[0] = 0x00;
        report.args[1] = zone;
        report.args[2] = mode_byte;
        report.args[3] = manual_flag;
        self.send_report(report).is_some()
    }

    fn set_zone_manual_fan(&mut self, zone: u8, manual_flag: u8) -> bool {
        if let Some((mode_byte, _)) = self.read_zone_fan_state(zone) {
            return self.set_zone_fan_state(zone, mode_byte, manual_flag);
        }
        false
    }

    fn read_stored_fan_setpoint(&mut self, zone: u8) -> Option<u16> {
        let mut report: RazerPacket = RazerPacket::new(0x0d, 0x81, 0x03);
        report.args[0] = 0x00;
        report.args[1] = zone;
        report.args[2] = 0x00;
        self.send_report(report)
            .map(|response| response.args[2] as u16 * 100)
    }

    pub fn read_fan_setting(&mut self) -> Option<u16> {
        let (_mode_byte, manual_flag) = self.read_zone_fan_state(0x01)?;
        if manual_flag == 0 {
            return Some(0);
        }
        self.read_stored_fan_setpoint(0x01)
    }

    fn set_power(&mut self, zone: u8) -> bool {
        let mut report: RazerPacket = RazerPacket::new(0x0d, 0x02, 0x04);
        report.args[0] = 0x00;
        report.args[1] = zone;
        report.args[2] = self.power;
        match self.fan_rpm {
            0 => report.args[3] = 0x00,
            _ => report.args[3] = 0x01,
        }
        if self.send_report(report).is_some() {
            return true;
        }

        false
    }

    pub fn get_cpu_boost(&mut self) -> u8 {
        let mut report: RazerPacket = RazerPacket::new(0x0d, 0x87, 0x03);
        report.args[0] = 0x00;
        report.args[1] = 0x01;
        report.args[2] = 0x00;
        if let Some(response) = self.send_report(report) {
            return response.args[2];
        }
        0
    }

    fn set_cpu_boost(&mut self, mut boost: u8) -> bool {
        let mut report: RazerPacket = RazerPacket::new(0x0d, 0x07, 0x03);
        if boost == 3 && !self.have_feature("boost".to_string()) {
            boost = 2;
        }
        report.args[0] = 0x00;
        report.args[1] = 0x01;
        report.args[2] = boost;
        if self.send_report(report).is_some() {
            return true;
        }

        false
    }

    fn get_gpu_boost(&mut self) -> u8 {
        let mut report: RazerPacket = RazerPacket::new(0x0d, 0x87, 0x03);
        report.args[0] = 0x00;
        report.args[1] = 0x02;
        report.args[2] = 0x00;
        if let Some(response) = self.send_report(report) {
            return response.args[2];
        }
        0
    }

    fn set_gpu_boost(&mut self, boost: u8) -> bool {
        let mut report: RazerPacket = RazerPacket::new(0x0d, 0x07, 0x03);
        report.args[0] = 0x00;
        report.args[1] = 0x02;
        report.args[2] = boost;
        if self.send_report(report).is_some() {
            return true;
        }
        false
    }

    pub fn set_power_mode(&mut self, mode: u8, cpu_boost: u8, gpu_boost: u8) -> bool {
        if mode == POWER_MODE_SILENT {
            self.power = power_mode_to_ec(mode);
            // Only a readback that names another mode counts as a rejection;
            // an EC that can't answer the read keeps the accepted write.
            let accepted = self.set_power(0x01)
                && self
                    .read_zone_fan_state(0x01)
                    .is_none_or(|(mode_byte, _)| mode_byte == EC_POWER_MODE_SILENT);
            if !accepted {
                // Older ECs have no Silent profile (and current ones only take
                // it on AC): emulate it with Custom and both boosts at Low
                // instead of leaving the previous profile. The config keeps
                // Silent so the next AC switch or restart retries 0x05.
                eprintln!("EC rejected Silent (0x05), falling back to Custom Low/Low");
                return self.set_power_mode(POWER_MODE_CUSTOM, 0, 0);
            }
            return self.set_power(0x02);
        } else if mode < POWER_MODE_CUSTOM {
            self.power = power_mode_to_ec(mode);
            self.set_power(0x01);
            self.set_power(0x02);
        } else if mode == POWER_MODE_CUSTOM {
            self.power = mode;
            self.fan_rpm = 0;
            self.get_power_mode(0x01);
            self.set_power(0x01);
            self.get_cpu_boost();
            self.set_cpu_boost(cpu_boost);
            self.get_gpu_boost();
            self.set_gpu_boost(gpu_boost);
            self.get_power_mode(0x02);
            self.set_power(0x02);
        }

        true
    }

    fn set_rpm(&mut self, zone: u8) -> bool {
        let mut report: RazerPacket = RazerPacket::new(0x0d, 0x01, 0x03);
        // Set fan RPM
        report.args[0] = 0x00;
        report.args[1] = zone;
        report.args[2] = self.fan_rpm;
        if self.send_report(report).is_some() {
            return true;
        }

        false
    }

    pub fn set_fan_rpm(&mut self, value: u16) -> bool {
        if value == 0 {
            self.fan_rpm = 0;
            let zone1 = self.set_zone_manual_fan(0x01, 0x00);
            let zone2 = self.set_zone_manual_fan(0x02, 0x00);
            return zone1 && zone2;
        }

        self.fan_rpm = self.clamp_fan(value);
        let zone1 = self.set_zone_manual_fan(0x01, 0x01);
        let zone2 = self.set_zone_manual_fan(0x02, 0x01);
        let fan1 = self.set_rpm(0x01);
        let fan2 = self.set_rpm(0x02);

        zone1 && zone2 && fan1 && fan2
    }

    /// Read fan RPM from EC hardware.
    /// Note: on many Razer models this returns the configured target,
    /// not measured tachometer RPM (no tach register exposed via USB HID).
    pub fn read_fan_rpm_from_ec(&mut self) -> u16 {
        if let Some(rpm) = self.read_stored_fan_setpoint(0x01) {
            return rpm;
        }
        self.fan_rpm as u16 * 100
    }

    /// Every device this project supports is classified as a "Blade laptop" in OpenRazer's
    /// `is_blade_laptop()` (razerkbd_driver.c) - every Blade Stealth/Pro/Advanced/Base/14-18
    /// variant, and the Razer Book (USB_DEVICE_ID_RAZER_BOOK_2020, this project's one non-Blade-
    /// named device), all match it. Per `razer_attr_write_logo_led_state`'s real Blade branch:
    /// mode 0 (Off) / 1 (On) send only the LED STATE command (id 0x00) - the LED EFFECT command
    /// (id 0x02) this used to always send first is for the non-Blade `else` branch and was
    /// getting the wrong value here (0x00/0x02 were never real "on"/"breathing" effect ids, just
    /// guessed). Mode 2 ("blink"/Breathing) is the reverse: the driver's Blade condition
    /// (`state == 0 || state == 1`) excludes it, so it takes the *effect* command instead, with
    /// the raw mode value as the effect id. Both branches hardcode transaction id 0xFF for logo
    /// specifically (unconditionally, in both branches) - unlike the id 0x0a standard-effect
    /// family, this isn't scoped to one model.
    ///
    /// This is confirmed byte-for-byte correct against the real driver, but on at least one
    /// Blade 15 Advanced Early 2022 unit the LED STATE command (id 0x00) is rejected outright by
    /// firmware as NOT_SUPPORTED, and the LED BRIGHTNESS command (id 0x03, otherwise proven
    /// working for keyboard backlight) is *also* rejected specifically for LOGO_LED - only the
    /// EFFECT command (id 0x02) is ever ACKed for this LED, and even that produces no visible
    /// change. No further command variant exists in the reference implementation to try (the
    /// mouse-only "logo matrix" extended-effect attributes have no keyboard/laptop equivalent) -
    /// this looks like a firmware-level limitation on that unit rather than a software bug.
    pub fn set_logo_led_state(&mut self, mode: u8) -> bool {
        let mut report: RazerPacket = if mode == 0 || mode == 1 {
            let mut r = RazerPacket::new(0x03, 0x00, 0x03);
            r.args[2] = self.clamp_u8(mode, 0x00, 0x01);
            r
        } else {
            let mut r = RazerPacket::new(0x03, 0x02, 0x03);
            r.args[2] = self.clamp_u8(mode, 0x00, 0x05);
            r
        };
        report.id = 0xFF;
        report.args[0] = RazerLaptop::VARSTORE;
        report.args[1] = RazerLaptop::LOGO_LED;
        self.send_report(report).is_some()
    }

    /// Keyboard backlight brightness (0-255). Uses the Blade laptop brightness command OpenRazer
    /// uses for every Blade (class 0x0E), falling back to the generic LED brightness command.
    /// On the Blade 15 Advanced (Early 2022) both drive the same setting.
    pub fn set_brightness(&mut self, brightness: u8) -> bool {
        let mut report = RazerPacket::new(0x0e, 0x04, 0x02);
        report.id = Self::LIGHTING_TRANSACTION_ID;
        report.args[0] = 0x01;
        report.args[1] = brightness;
        if self.send_report(report).is_some() {
            return true;
        }

        let mut report: RazerPacket = RazerPacket::new(0x03, 0x03, 0x03);
        report.id = Self::LIGHTING_TRANSACTION_ID;
        report.args[0] = RazerLaptop::VARSTORE;
        report.args[1] = RazerLaptop::BACKLIGHT_LED;
        report.args[2] = brightness;
        self.send_report(report).is_some()
    }

    /// Current backlight brightness (0-255) as the keyboard reports it, which also reflects
    /// changes made with the Fn brightness keys.
    pub fn read_brightness(&mut self) -> Option<u8> {
        let mut report = RazerPacket::new(0x0e, 0x84, 0x02);
        report.id = Self::LIGHTING_TRANSACTION_ID;
        report.args[0] = 0x01;
        if let Some(r) = self.send_report(report) {
            return Some(r.args[1]);
        }

        let mut report = RazerPacket::new(0x03, 0x83, 0x03);
        report.id = Self::LIGHTING_TRANSACTION_ID;
        report.args[0] = RazerLaptop::VARSTORE;
        report.args[1] = RazerLaptop::BACKLIGHT_LED;
        self.send_report(report).map(|r| r.args[2])
    }

    /// Keyboard firmware version, e.g. "v1.2".
    pub fn read_firmware_version(&mut self) -> Option<String> {
        let mut report = RazerPacket::new(0x00, 0x81, 0x02);
        report.id = Self::LIGHTING_TRANSACTION_ID;
        self.send_report(report)
            .map(|r| format!("v{}.{}", r.args[0], r.args[1]))
    }

    /// Physical keyboard layout id (see `service::keyboard_layout_name`).
    pub fn read_keyboard_layout(&mut self) -> Option<u8> {
        let mut report = RazerPacket::new(0x00, 0x86, 0x02);
        report.id = Self::LIGHTING_TRANSACTION_ID;
        self.send_report(report).map(|r| r.args[0])
    }

    /// Serial number stored in the keyboard. Blade laptops generally leave this blank (all 0xFF)
    /// and keep the serial in the BIOS instead, which only root can read - None then.
    pub fn read_serial(&mut self) -> Option<String> {
        let mut report = RazerPacket::new(0x00, 0x82, 0x16);
        report.id = Self::LIGHTING_TRANSACTION_ID;
        let r = self.send_report(report)?;
        let serial: String = r.args[..22]
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b as char)
            .collect();
        let serial = serial.trim().to_string();
        (!serial.is_empty() && serial.chars().all(|c| c.is_ascii_graphic())).then_some(serial)
    }

    pub fn set_bho(&mut self, is_on: bool, threshold: u8) -> bool {
        if !self.have_feature("bho".to_string()) {
            return false;
        }

        let mut report = RazerPacket::new(0x07, 0x12, 0x01);
        report.args[0] = bho_to_byte(is_on, threshold);

        self.send_report(report).is_some_and(|r| {
            println!("Response Packet:\n{:#?}", r);
            true
        })
    }

    fn send_report(&mut self, mut report: RazerPacket) -> Option<RazerPacket> {
        let debug = std::env::var("RAZER_HID_DEBUG").is_ok();
        if debug {
            eprintln!(
                "HID>> class={:#x} id={:#x} tid={:#x} size={} args={:?}",
                report.command_class,
                report.command_id,
                report.id,
                report.data_size,
                &report.args[..8]
            );
        }
        let mut temp_buf: [u8; 91] = [0x00; 91];
        for _ in 0..3 {
            match self
                .device
                .send_feature_report(report.calc_crc().as_slice())
            {
                Ok(_) => {
                    thread::sleep(time::Duration::from_micros(1000));
                    match self.device.get_feature_report(&mut temp_buf) {
                        Ok(size) => {
                            if size == 91 {
                                match bincode::deserialize::<RazerPacket>(&temp_buf) {
                                    Ok(response) => {
                                        if debug {
                                            eprintln!(
                                                "HID<< status={:#x} class={:#x} id={:#x} args={:?}",
                                                response.status,
                                                response.command_class,
                                                response.command_id,
                                                &response.args[..8]
                                            );
                                        }
                                        // when request bho status the response command id is different from the request command id...
                                        if response.command_id == 0x92 {
                                            return Some(response);
                                        }

                                        if response.remaining_packets != report.remaining_packets
                                            || response.command_class != report.command_class
                                            || response.command_id != report.command_id
                                        {
                                            eprintln!("Response doesn't match request");
                                        } else if response.status
                                            == RazerPacket::RAZER_CMD_SUCCESSFUL
                                        {
                                            return Some(response);
                                        }
                                        if response.status == RazerPacket::RAZER_CMD_NOT_SUPPORTED {
                                            eprintln!("Command not supported");
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("Error: {}", e);
                                    }
                                }
                            } else {
                                eprintln!("Invalid report length: {:?}", size);
                            }
                        }
                        Err(e) => {
                            eprintln!("Error: {}", e);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                }
            };
        }

        thread::sleep(time::Duration::from_micros(8000));
        None
    }
}

/// Power profile indices as stored in the config and shown by the frontends
/// (0=Balanced, 1=Gaming, 2=Creator, 3=Silent, 4=Custom).
const POWER_MODE_SILENT: u8 = 3;
const POWER_MODE_CUSTOM: u8 = 4;
/// EC wire value for Silent. Synapse sends 0x05 (Blade 16 2025 USB captures,
/// Blade 14 2023 #39); 0x03 is Battery Saver on current ECs and an undefined
/// mode on older ones, so the profile index must not be sent as-is.
const EC_POWER_MODE_SILENT: u8 = 0x05;

/// Translate a profile index into the byte the EC expects in `0x0d/0x02`.
fn power_mode_to_ec(mode: u8) -> u8 {
    match mode {
        POWER_MODE_SILENT => EC_POWER_MODE_SILENT,
        other => other,
    }
}

// Top bit flags whether battery health optimization is on; the low 7 bits are the threshold.
fn bho_to_byte(is_on: bool, threshold: u8) -> u8 {
    if is_on {
        return threshold | 0b1000_0000;
    }
    threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_is_sent_as_ec_mode_5() {
        assert_eq!(power_mode_to_ec(POWER_MODE_SILENT), 0x05);
    }

    #[test]
    fn other_profiles_keep_their_wire_value() {
        assert_eq!(power_mode_to_ec(0), 0);
        assert_eq!(power_mode_to_ec(1), 1);
        assert_eq!(power_mode_to_ec(2), 2);
        assert_eq!(power_mode_to_ec(POWER_MODE_CUSTOM), 4);
    }

    #[test]
    fn no_profile_is_sent_as_battery_saver() {
        assert!((0..=POWER_MODE_CUSTOM).all(|mode| power_mode_to_ec(mode) != 0x03));
    }

    fn logo_state_packet() -> RazerPacket {
        let mut report = RazerPacket::new(0x03, 0x00, 0x03);
        report.args[0] = 0x01;
        report.args[1] = 0x04;
        report.args[2] = 0x01;
        report
    }

    #[test]
    fn crc_matches_openrazer_range_and_is_in_the_first_send() {
        let mut report = logo_state_packet();
        let buf = report.calc_crc();
        assert_eq!(buf.len(), 91);
        // data_size (0x03) and class (0x03) cancel, cmd is 0x00, and args 01 04 01 reduce to 0x04.
        let expected = 0x04;
        assert_eq!(buf[89], expected);
        assert_eq!(report.crc, expected);
    }

    #[test]
    fn crc_ignores_transaction_id() {
        let mut a = logo_state_packet();
        let mut b = logo_state_packet();
        b.id = 0xFF;
        assert_eq!(a.calc_crc()[89], b.calc_crc()[89]);
    }

    #[test]
    fn crc_covers_the_last_argument_byte() {
        let mut a = logo_state_packet();
        let mut b = logo_state_packet();
        b.args[79] = 0x5A;
        assert_eq!(a.calc_crc()[89] ^ b.calc_crc()[89], 0x5A);
    }

    #[test]
    fn crc_is_stable_across_retries() {
        let mut report = logo_state_packet();
        let first = report.calc_crc();
        assert_eq!(first, report.calc_crc());
    }
}
