use std::fs::{self, File};
use std::io::{ErrorKind, Read};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::kbd::board;

/// Key positions queued by the capture thread and consumed by the Ripple effect each frame.
pub type Presses = Arc<Mutex<Vec<(usize, usize)>>>;

/// Presses beyond this many unconsumed ones are dropped (the effect drains the queue every frame,
/// so this only matters if rendering stalls).
const MAX_PENDING: usize = 32;
/// The laptop keyboard's key reports arrive on this USB interface (interface 0 is a boot keyboard
/// that stays silent; interface 2 is pointer-like).
const KEY_INTERFACE: u32 = 1;
const POLL_MS: i32 = 100;
const REOPEN_DELAY: Duration = Duration::from_secs(1);

/// Watches the Razer keyboard for key presses while Ripple is active, so each press can start a
/// ripple at that key's LED.
///
/// Reads the keyboard's own raw HID input reports, which the logged-in user can already read via
/// the same udev ACL used for lighting - no `input` group membership or evdev access is needed.
/// Only the grid position of each newly pressed key is kept; key codes are never logged, stored,
/// or sent anywhere, and the device is closed as soon as the capture is dropped.
pub struct KeyCapture {
    running: Arc<AtomicBool>,
    pub presses: Presses,
}

impl Drop for KeyCapture {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

impl KeyCapture {
    /// Starts watching the keyboard with USB product ID `pid`. None if its key interface can't be
    /// found or opened.
    pub fn start(pid: u16) -> Option<KeyCapture> {
        let file = open_key_interface(pid)?;
        let running = Arc::new(AtomicBool::new(true));
        let presses: Presses = Arc::new(Mutex::new(Vec::new()));

        let thread_running = running.clone();
        let thread_presses = presses.clone();
        thread::spawn(move || capture_loop(file, pid, thread_running, thread_presses));

        Some(KeyCapture { running, presses })
    }
}

/// Opens the Razer hidraw node for `pid` on the key-report interface. Node numbers change across
/// reboots and resume, so it's located by USB IDs and interface number rather than path.
fn open_key_interface(pid: u16) -> Option<File> {
    let hid_id = format!("HID_ID=0003:00001532:{pid:08X}");
    for entry in fs::read_dir("/sys/class/hidraw").ok()?.flatten() {
        let dev = entry.path().join("device");
        let Ok(uevent) = fs::read_to_string(dev.join("uevent")) else {
            continue;
        };
        if !uevent.lines().any(|l| l.eq_ignore_ascii_case(&hid_id)) {
            continue;
        }
        let iface = fs::read_to_string(dev.join("../bInterfaceNumber"))
            .ok()
            .and_then(|s| u32::from_str_radix(s.trim(), 16).ok());
        if iface == Some(KEY_INTERFACE) {
            return File::open(format!("/dev/{}", entry.file_name().to_str()?)).ok();
        }
    }
    None
}

fn capture_loop(mut file: File, pid: u16, running: Arc<AtomicBool>, presses: Presses) {
    let mut parser = ReportParser::default();
    let mut buf = [0u8; 64];
    while running.load(Ordering::SeqCst) {
        let mut pfd = libc::pollfd {
            fd: file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `pfd` is a valid pollfd for an fd owned by `file`, which outlives the call.
        let ready = unsafe { libc::poll(&mut pfd, 1, POLL_MS) };
        if ready == 0
            || (ready < 0 && std::io::Error::last_os_error().kind() == ErrorKind::Interrupted)
        {
            continue;
        }
        let read = if ready > 0 && pfd.revents & libc::POLLIN != 0 {
            file.read(&mut buf)
        } else {
            Err(ErrorKind::BrokenPipe.into())
        };
        match read {
            Ok(n) if n > 0 => {
                let new = parser.new_presses(&buf[..n]);
                if !new.is_empty()
                    && let Ok(mut p) = presses.lock()
                {
                    p.extend(new);
                    let excess = p.len().saturating_sub(MAX_PENDING);
                    p.drain(..excess);
                }
            }
            _ => {
                // The node went away (suspend/resume, USB reset) - wait for it to come back.
                parser = ReportParser::default();
                loop {
                    thread::sleep(REOPEN_DELAY);
                    if !running.load(Ordering::SeqCst) {
                        return;
                    }
                    if let Some(f) = open_key_interface(pid) {
                        file = f;
                        break;
                    }
                }
            }
        }
    }
}

/// Turns HID input reports into newly pressed keys. Reports list every key currently held, so a
/// press is a key that appears in a report but wasn't in the previous one of the same kind.
#[derive(Default)]
struct ReportParser {
    modifiers: u8,
    keys: Vec<u8>,
    vendor: Vec<u8>,
}

impl ReportParser {
    fn new_presses(&mut self, report: &[u8]) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        match report.first() {
            // Keyboard: [1][modifier bits][up to 14 usage codes]
            Some(1) if report.len() >= 2 => {
                let mods = report[1];
                for bit in 0..8 {
                    if mods & !self.modifiers & (1 << bit) != 0
                        && let Some(pos) = modifier_position(bit)
                    {
                        out.push(pos);
                    }
                }
                self.modifiers = mods;
                let keys: Vec<u8> = report[2..].iter().copied().filter(|&c| c > 3).collect();
                out.extend(
                    keys.iter()
                        .filter(|c| !self.keys.contains(c))
                        .filter_map(|&c| usage_position(c)),
                );
                self.keys = keys;
            }
            // Vendor key arrays (Fn and friends): [4 or 5][up to 15 codes]
            Some(4 | 5) => {
                let codes: Vec<u8> = report[1..].iter().copied().filter(|&c| c != 0).collect();
                out.extend(
                    codes
                        .iter()
                        .filter(|c| !self.vendor.contains(c))
                        .filter_map(|&c| vendor_position(c)),
                );
                self.vendor = codes;
            }
            _ => {}
        }
        out.retain(|&(r, c)| r < board::ROWS && c < board::KEYS_PER_ROW);
        out
    }
}

// Physical key → LED (row, col), measured on the Blade 15 Advanced (Early 2022) by lighting one
// LED at a time and pressing the key under it. Column 0 has no keys on this layout. Other models
// share the report format; any key not listed here simply doesn't start a ripple.

fn usage_position(usage: u8) -> Option<(usize, usize)> {
    Some(match usage {
        0x29 => (0, 1),                                  // Esc
        0x3A..=0x45 => (0, 2 + (usage - 0x3A) as usize), // F1-F12
        0x4C => (0, 14),                                 // Delete
        0x35 => (1, 1),                                  // `
        0x1E..=0x27 => (1, 2 + (usage - 0x1E) as usize), // 1-0
        0x2D => (1, 12),                                 // -
        0x2E => (1, 13),                                 // =
        0x2A => (1, 15),                                 // Backspace
        0x2B => (2, 1),                                  // Tab
        0x14 => (2, 2),                                  // Q
        0x1A => (2, 3),                                  // W
        0x08 => (2, 4),                                  // E
        0x15 => (2, 5),                                  // R
        0x17 => (2, 6),                                  // T
        0x1C => (2, 7),                                  // Y
        0x18 => (2, 8),                                  // U
        0x0C => (2, 9),                                  // I
        0x12 => (2, 10),                                 // O
        0x13 => (2, 11),                                 // P
        0x2F => (2, 12),                                 // [
        0x30 => (2, 13),                                 // ]
        0x31 => (2, 15),                                 // \
        0x39 => (3, 1),                                  // Caps Lock
        0x04 => (3, 2),                                  // A
        0x16 => (3, 3),                                  // S
        0x07 => (3, 4),                                  // D
        0x09 => (3, 5),                                  // F
        0x0A => (3, 6),                                  // G
        0x0B => (3, 7),                                  // H
        0x0D => (3, 8),                                  // J
        0x0E => (3, 9),                                  // K
        0x0F => (3, 10),                                 // L
        0x33 => (3, 11),                                 // ;
        0x34 => (3, 12),                                 // '
        0x28 => (3, 15),                                 // Enter
        0x1D => (4, 3),                                  // Z
        0x1B => (4, 4),                                  // X
        0x06 => (4, 5),                                  // C
        0x19 => (4, 6),                                  // V
        0x05 => (4, 7),                                  // B
        0x11 => (4, 8),                                  // N
        0x10 => (4, 9),                                  // M
        0x36 => (4, 10),                                 // ,
        0x37 => (4, 11),                                 // .
        0x38 => (4, 12),                                 // /
        0x2C => (5, 7),                                  // Space (centre of the bar)
        0x50 => (5, 12),                                 // Left
        0x52 => (5, 13),                                 // Up
        0x4F => (5, 14),                                 // Right
        0x51 => (5, 15),                                 // Down
        _ => return None,
    })
}

/// Modifier bit (0 = Left Ctrl ... 7 = Right Super) → LED.
fn modifier_position(bit: u8) -> Option<(usize, usize)> {
    Some(match bit {
        0 => (5, 1),  // Left Ctrl
        1 => (4, 1),  // Left Shift
        2 => (5, 5),  // Left Alt
        3 => (5, 3),  // Super
        4 => (5, 11), // Right Ctrl
        5 => (4, 15), // Right Shift
        6 => (5, 9),  // Right Alt
        _ => return None,
    })
}

fn vendor_position(code: u8) -> Option<(usize, usize)> {
    Some(match code {
        0x0A => (5, 2),  // Fn
        0x0B => (5, 10), // Right-hand Fn/menu key
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_report(mods: u8, keys: &[u8]) -> Vec<u8> {
        let mut r = vec![1, mods];
        r.extend_from_slice(keys);
        r.resize(16, 0);
        r
    }

    #[test]
    fn only_new_keys_count_as_presses() {
        let mut p = ReportParser::default();
        assert_eq!(p.new_presses(&key_report(0, &[0x04])), vec![(3, 2)]); // A down
        assert_eq!(p.new_presses(&key_report(0, &[0x04, 0x16])), vec![(3, 3)]); // S added
        assert!(p.new_presses(&key_report(0, &[0x16])).is_empty()); // A released
        assert!(p.new_presses(&key_report(0, &[])).is_empty());
        assert_eq!(p.new_presses(&key_report(0, &[0x04])), vec![(3, 2)]); // A again
    }

    #[test]
    fn modifiers_and_ranges() {
        let mut p = ReportParser::default();
        assert_eq!(p.new_presses(&key_report(0b10, &[])), vec![(4, 1)]); // Left Shift
        assert!(p.new_presses(&key_report(0b10, &[])).is_empty()); // still held
        assert_eq!(p.new_presses(&key_report(0, &[0x45])), vec![(0, 13)]); // F12
        assert_eq!(p.new_presses(&key_report(0, &[0x27])), vec![(1, 11)]); // 0
    }

    #[test]
    fn vendor_and_ignored_reports() {
        let mut p = ReportParser::default();
        assert_eq!(p.new_presses(&[4, 0x0A, 0, 0]), vec![(5, 2)]);
        assert!(p.new_presses(&[4, 0x0A, 0, 0]).is_empty());
        assert!(p.new_presses(&[2, 0xE9, 0x00]).is_empty()); // consumer (volume) report
        assert!(p.new_presses(&key_report(0, &[0x01])).is_empty()); // rollover error code
    }
}
