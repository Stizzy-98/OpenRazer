use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use rustfft::FftPlanner;
use rustfft::num_complex::Complex32;

pub const BANDS: usize = 16;
const SAMPLE_RATE: f32 = 44100.0;
// Shrinking this to 512 to chase responsiveness was a real bug, not a fix: live debug logging
// with real music showed the bottom 4 bands permanently pegged at ~1.0. At 512 samples, bin_hz
// (~86Hz) is wider than several of the low log-spaced bands, so bands 0-3 all collapsed onto the
// SAME single FFT bin - and since bass naturally carries the most raw energy in real audio, that
// one bin is almost always the frame's loudest, making those columns read "100% of max" nearly
// constantly. 2048 gives ~21.5Hz/bin, fine enough that every band maps to a genuinely distinct
// bin range (verified: band0=[2,3), band1=[3,4), no overlap) - the real fix for "stuck lingering"
// was this, not decay speed (which was already mathematically instant at decay=0).
const WINDOW_SIZE: usize = 2048;
const LOW_HZ: f32 = 50.0;
const HIGH_HZ: f32 = 16000.0;
// Read in small overlapping hops so the spectrum updates continuously rather than in big
// discrete jumps.
const HOP_SIZE: usize = 256; // ~5.8ms at 44.1kHz
// The log-compression curve below (from the real reference implementation) only produces
// visible, non-saturated output for a fairly narrow input window (roughly 0.25-1.1) - live
// calibration on this exact pipeline showed raw gained values reaching up to ~10 at the old
// default (sensitivity=1.0, no base gain), which is why the entire keyboard was maxing out to
// full brightness regardless of what was actually playing. This brings loud moments back down
// into the part of the curve where they read as bright but not universally clipped white.
const BASE_GAIN: f32 = 0.08;

/// User-configurable knobs, live-updatable while capture is already running (the GUI sends a new
/// `SetSoundBarEffect` on every Apply, which just updates this in place rather than needing to
/// restart the capture subprocess). These map directly onto the "amplitude" and "decay" controls
/// in FIX94/KeyboardVisualizer (a real, widely-used open-source Chroma audio visualizer whose
/// source was checked directly) - a manual gain knob plus per-frame peak-hold decay, not an
/// auto-normalizing or onset/baseline scheme.
#[derive(Clone, Copy)]
pub struct AudioConfig {
    /// Manual gain applied to raw magnitude before the compression curve, matching that
    /// reference's "amplitude" - there's no auto-normalization, so the user dials this to their
    /// own volume level (higher = smaller/quieter signals register).
    pub sensitivity: f32,
    /// Per-hop decay retention (0.0 = falls to black almost instantly, close to 1.0 = lingers
    /// longer). Every hop first decays every band's current value by this factor, then raises it
    /// back up only if the new incoming value is higher - a real peak-hold-with-decay, not a
    /// blended average.
    pub decay: f32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        AudioConfig {
            sensitivity: 1.0,
            decay: 0.3,
        }
    }
}

/// Captures system audio via `parec` (already installed as part of pulseaudio-utils, works
/// against PipeWire's pulse-compat layer here) and continuously writes 16 per-frequency-band
/// levels (0.0-1.0, one per keyboard column) into a shared array the SoundBar effect reads every
/// render tick - a real spectrum analyzer, matching how Razer's own Audio Meter effect actually
/// works (confirmed against a real reference video: each column is a different frequency band,
/// independently reaching a different height, changing rapidly). Shelling out to `parec` avoids
/// binding to PipeWire/PulseAudio's C API from Rust entirely - simpler and more robust than a
/// full audio-IO crate for this.
pub struct AudioCapture {
    child: Child,
    running: Arc<AtomicBool>,
    pub levels: Arc<Mutex<[f32; BANDS]>>,
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Resolves the current default sink's monitor source name via `pactl`. The `@DEFAULT_MONITOR@`
/// magic alias did not work with `parec`'s `-d` flag when tested directly on this machine - the
/// literal monitor source name is required, so this is resolved fresh each time capture starts
/// (following whatever output device is actually active, e.g. speakers vs. headphones).
fn default_monitor_source() -> Option<String> {
    let out = Command::new("pactl")
        .arg("get-default-sink")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sink = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if sink.is_empty() {
        return None;
    }
    Some(format!("{sink}.monitor"))
}

/// Compensation weight per band (1.0x at the lowest band, up to 4.0x at the highest) - counters
/// real audio's naturally bass-heavy raw magnitude spectrum so treble bands get a fair chance to
/// show real activity instead of the low end structurally dominating every frame.
fn band_weights() -> [f32; BANDS] {
    let mut w = [1.0f32; BANDS];
    for (i, wi) in w.iter_mut().enumerate() {
        *wi = 1.0 + (i as f32 / (BANDS as f32 - 1.0)) * 3.0;
    }
    w
}

/// Bucket boundaries (in FFT bins) for BANDS log-spaced bands between LOW_HZ and HIGH_HZ - narrow
/// at the low end, wide at the high end, matching how audio frequency is actually perceived. The
/// low third of columns reads as bass, the middle third as mids, the high third as treble.
fn band_bin_ranges() -> [(usize, usize); BANDS] {
    let bin_hz = SAMPLE_RATE / WINDOW_SIZE as f32;
    let max_bin = WINDOW_SIZE / 2;
    let mut ranges = [(0usize, 0usize); BANDS];
    let log_low = LOW_HZ.ln();
    let log_high = HIGH_HZ.ln();
    for (i, range) in ranges.iter_mut().enumerate() {
        let f_lo = (log_low + (log_high - log_low) * (i as f32 / BANDS as f32)).exp();
        let f_hi = (log_low + (log_high - log_low) * ((i + 1) as f32 / BANDS as f32)).exp();
        let bin_lo = ((f_lo / bin_hz) as usize).clamp(1, max_bin - 1);
        let bin_hi = ((f_hi / bin_hz) as usize).clamp(bin_lo + 1, max_bin);
        *range = (bin_lo, bin_hi);
    }
    ranges
}

impl AudioCapture {
    pub fn start(config: Arc<Mutex<AudioConfig>>) -> Option<AudioCapture> {
        let monitor = default_monitor_source()?;
        // --latency-msec is essential, not cosmetic: without it, PipeWire's pulse-compat layer
        // defaults to a much larger buffer, so `parec` delivers audio in big bursts every ~600ms
        // instead of a continuous stream - confirmed directly (a raw parec test with this flag
        // showed no read gaps over 20ms in 4 seconds; without it, the render side saw the exact
        // same set of levels repeat for ~600ms at a time before jumping). This is what was making
        // the display look like a fixed line instead of continuously moving.
        let mut child = Command::new("parec")
            .args([
                "-d",
                &monitor,
                "--format=s16le",
                "--rate=44100",
                "--channels=1",
                "--latency-msec=10",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let stdout = child.stdout.take()?;

        let running = Arc::new(AtomicBool::new(true));
        let levels = Arc::new(Mutex::new([0.0f32; BANDS]));

        let thread_running = running.clone();
        let thread_levels = levels.clone();
        thread::spawn(move || {
            capture_loop(stdout, thread_running, thread_levels, config);
        });

        Some(AudioCapture {
            child,
            running,
            levels,
        })
    }
}

fn capture_loop(
    mut stdout: impl Read,
    running: Arc<AtomicBool>,
    levels: Arc<Mutex<[f32; BANDS]>>,
    config: Arc<Mutex<AudioConfig>>,
) {
    let ranges = band_bin_ranges();
    let band_weights = band_weights();
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(WINDOW_SIZE);

    let hann: Vec<f32> = (0..WINDOW_SIZE)
        .map(|i| {
            0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (WINDOW_SIZE - 1) as f32).cos()
        })
        .collect();

    let mut ring = vec![0.0f32; WINDOW_SIZE];
    let mut hop_buf = vec![0u8; HOP_SIZE * 2];
    // Peak-hold-with-decay per band, matching the reference exactly: every hop decays first, then
    // only rises back up if the new incoming value is higher.
    let mut pulse = [0.0f32; BANDS];

    while running.load(Ordering::SeqCst) {
        if stdout.read_exact(&mut hop_buf).is_err() {
            break; // parec exited or pipe closed
        }

        ring.copy_within(HOP_SIZE.., 0);
        for (i, b) in hop_buf.as_chunks::<2>().0.iter().enumerate() {
            let sample = i16::from_le_bytes(*b) as f32 / 32768.0;
            ring[WINDOW_SIZE - HOP_SIZE + i] = sample;
        }

        let mut buffer: Vec<Complex32> = ring
            .iter()
            .enumerate()
            .map(|(i, &s)| Complex32::new(s * hann[i], 0.0))
            .collect();
        fft.process(&mut buffer);

        let AudioConfig { sensitivity, decay } = config.lock().map(|c| *c).unwrap_or_default();

        for (i, (lo, hi)) in ranges.iter().enumerate() {
            let mag: f32 =
                buffer[*lo..*hi].iter().map(|c| c.norm()).sum::<f32>() / (*hi - *lo) as f32;
            // Real music's raw FFT magnitude is naturally bass-heavy - without this, the lowest
            // bands would almost always dominate simply from how audio energy is distributed
            // across frequency, regardless of resolution. Boosting higher bands compensates so
            // every column gets a fair chance to show real activity.
            let gained = mag * band_weights[i] * sensitivity * BASE_GAIN;

            // Log-linear compression curve, taken directly from FIX94/KeyboardVisualizer's real,
            // working implementation - boosts quiet content into visibility without an
            // auto-normalizing scheme (which is what made earlier attempts either flatten out the
            // relative shape or need a slow-adapting ceiling that lagged behind real changes).
            let compressed = if gained > 1e-6 {
                (0.5 * (1.1 * gained).log10() + 0.9 * gained).clamp(0.0, 1.0)
            } else {
                0.0
            };

            // Peak-hold with decay, same as the reference: decay first, then only rise back up if
            // the new value is higher - a real per-band bounce, not a blended average.
            pulse[i] = (pulse[i] * decay).max(compressed);
        }

        if let Ok(mut l) = levels.lock() {
            *l = pulse;
        }
    }
}
