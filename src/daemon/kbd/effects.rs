use super::*;

///
/// STATIC KEYBOARD EFFECT
/// 1 colour, simple
///

#[derive(Copy, Clone)]
pub struct Static {
    kbd: board::KeyboardData,
    args: [u8; 3],
}

impl Effect for Static {
    fn new(args: Vec<u8>) -> Box<dyn Effect>
    where
        Self: Sized,
    {
        let r = *args.get(0).unwrap_or(&0);
        let g = *args.get(1).unwrap_or(&0);
        let b = *args.get(2).unwrap_or(&0);
        let mut kbd = board::KeyboardData::new();
        kbd.set_kbd_colour(r, g, b);
        let s = Static {
            kbd,
            args: [r, g, b],
        };
        Box::new(s)
    }

    fn update(&mut self) -> board::KeyboardData {
        self.kbd
    }

    fn get_name() -> &'static str
    where
        Self: Sized,
    {
        "Static"
    }

    fn get_varargs(&mut self) -> &[u8] {
        &self.args
    }

    fn clone_box(&self) -> Box<dyn Effect> {
        Box::new(*self)
    }

    fn save(&mut self) -> EffectSave {
        EffectSave {
            args: self.args.to_vec(),
            name: String::from("Static"),
        }
    }

    fn get_state(&mut self) -> Vec<u8> {
        self.kbd.get_curr_state()
    }
}

///
/// STATIC_BLEND KEYBOARD EFFECT
/// 2 colours forming a gradient
///

#[derive(Copy, Clone)]
pub struct StaticGradient {
    kbd: board::KeyboardData,
    args: [u8; 6],
}

impl Effect for StaticGradient {
    fn new(args: Vec<u8>) -> Box<dyn Effect>
    where
        Self: Sized,
    {
        let mut kbd = board::KeyboardData::new();
        let args: [u8; 6] = [
            *args.get(0).unwrap_or(&0),
            *args.get(1).unwrap_or(&0),
            *args.get(2).unwrap_or(&0),
            *args.get(3).unwrap_or(&0),
            *args.get(4).unwrap_or(&0),
            *args.get(5).unwrap_or(&0),
        ];
        let mut c1 = board::AnimatorKeyColour::new_u(args[0], args[1], args[2]);
        let c2 = board::AnimatorKeyColour::new_u(args[3], args[4], args[5]);
        let delta = (c2 - c1).divide(14.0);
        for i in 0..15 {
            let clamped = c1.get_clamped_colour();
            kbd.set_col_colour(i, clamped.red, clamped.green, clamped.blue);
            c1 += delta;
        }

        Box::new(StaticGradient { kbd, args })
    }

    fn update(&mut self) -> board::KeyboardData {
        self.kbd // Nothing to update
    }

    fn get_name() -> &'static str
    where
        Self: Sized,
    {
        "Static Gradient"
    }

    fn get_varargs(&mut self) -> &[u8] {
        &self.args
    }

    fn clone_box(&self) -> Box<dyn Effect> {
        Box::new(*self)
    }

    fn save(&mut self) -> EffectSave {
        EffectSave {
            args: self.args.to_vec(),
            name: String::from("Static Gradient"),
        }
    }

    fn get_state(&mut self) -> Vec<u8> {
        self.kbd.get_curr_state()
    }
}

///
/// STATIC_BLEND KEYBOARD EFFECT
/// 2 colours forming a gradient, animated across the keyboard
///
pub struct WaveGradient {
    kbd: board::KeyboardData,
    args: [u8; 6],
    colour_band: Vec<board::AnimatorKeyColour>,
}

impl Effect for WaveGradient {
    fn new(args: Vec<u8>) -> Box<dyn Effect>
    where
        Self: Sized,
    {
        let args: [u8; 6] = [
            *args.get(0).unwrap_or(&0),
            *args.get(1).unwrap_or(&0),
            *args.get(2).unwrap_or(&0),
            *args.get(3).unwrap_or(&0),
            *args.get(4).unwrap_or(&0),
            *args.get(5).unwrap_or(&0),
        ];
        let mut wave = WaveGradient {
            kbd: board::KeyboardData::new(),
            args,
            colour_band: vec![],
        };
        let mut c1 = board::AnimatorKeyColour::new_u(args[0], args[1], args[2]);
        let mut c2 = board::AnimatorKeyColour::new_u(args[3], args[4], args[5]);
        let c_delta = (c2 - c1).divide(15.0);
        for _ in 0..15 {
            wave.colour_band.push(c1);
            c1 += c_delta;
        }
        for _ in 0..15 {
            wave.colour_band.push(c2);
            c2 -= c_delta;
        }
        Box::new(wave)
    }

    fn update(&mut self) -> board::KeyboardData {
        for i in 0..15 {
            let c = self.colour_band[i].get_clamped_colour();
            self.kbd.set_col_colour(i, c.red, c.green, c.blue);
        }
        self.colour_band.rotate_right(1);
        self.kbd
    }

    fn get_name() -> &'static str
    where
        Self: Sized,
    {
        "Wave Gradient"
    }

    fn get_varargs(&mut self) -> &[u8] {
        &self.args
    }

    fn clone_box(&self) -> Box<dyn Effect> {
        Box::new(self.clone())
    }

    fn save(&mut self) -> EffectSave {
        EffectSave {
            args: self.args.to_vec(),
            name: String::from("Wave Gradient"),
        }
    }

    fn get_state(&mut self) -> Vec<u8> {
        self.kbd.get_curr_state()
    }
}

impl Clone for WaveGradient {
    fn clone(&self) -> Self {
        WaveGradient {
            kbd: self.kbd,
            args: self.args,
            colour_band: self.colour_band.to_vec(),
        }
    }
}

///
/// BREATHING (1 Colour) KEYBOARD EFFECT
/// 1 colour, fading in and out
///
#[derive(Copy, Clone)]
pub struct BreathSingle {
    args: [u8; 4],
    kbd: board::KeyboardData,
    step_duration_ms: u128,
    static_start_ms: u128,
    curr_step: u8, // Step 0 = Off, 1 = increasing, 2 = On, 3 = decreasing
    target_colour: board::AnimatorKeyColour,
    current_colour: board::AnimatorKeyColour,
    animator_step_colour: board::AnimatorKeyColour,
}

impl Effect for BreathSingle {
    fn new(args: Vec<u8>) -> Box<dyn Effect> {
        let r = *args.get(0).unwrap_or(&0);
        let g = *args.get(1).unwrap_or(&0);
        let b = *args.get(2).unwrap_or(&0);
        let d = *args.get(3).unwrap_or(&10);
        let mut k = board::KeyboardData::new();
        let cycle_duration_ms = d as f32 * 100.0;
        k.set_kbd_colour(0, 0, 0); // Sets all keyboard lights off initially
        Box::new(BreathSingle {
            args: [r, g, b, d],
            kbd: k,
            step_duration_ms: cycle_duration_ms as u128,
            static_start_ms: get_millis(),
            curr_step: 0,
            target_colour: board::AnimatorKeyColour::new_u(r, g, b),
            current_colour: board::AnimatorKeyColour::new_u(0, 0, 0),
            animator_step_colour: board::AnimatorKeyColour::new_f(
                r as f32 / (cycle_duration_ms / ANIMATION_SLEEP_MS as f32),
                g as f32 / (cycle_duration_ms / ANIMATION_SLEEP_MS as f32),
                b as f32 / (cycle_duration_ms / ANIMATION_SLEEP_MS as f32),
            ),
        })
    }

    fn update(&mut self) -> board::KeyboardData {
        match self.curr_step {
            0 => {
                self.current_colour = board::AnimatorKeyColour::new_u(0, 0, 0);
                if get_millis() - self.static_start_ms >= self.step_duration_ms {
                    self.curr_step += 1;
                }
            }
            1 => {
                // Increasing
                self.current_colour += self.animator_step_colour;
                if self.current_colour >= self.target_colour {
                    self.curr_step += 1;
                    self.static_start_ms = get_millis();
                }
            }
            2 => {
                self.current_colour = self.target_colour;
                if get_millis() - self.static_start_ms >= self.step_duration_ms {
                    self.curr_step += 1;
                }
            }
            3 => {
                // Decreasing
                self.current_colour -= self.animator_step_colour;
                let target = board::AnimatorKeyColour::new_u(0, 0, 0);
                if self.current_colour <= target {
                    self.curr_step = 0;
                    self.static_start_ms = get_millis();
                }
            }
            _ => {} // Unknown state? Ignore
        }
        let col = self.current_colour.get_clamped_colour();
        self.kbd.set_kbd_colour(col.red, col.green, col.blue); // Cast back to u8
        self.kbd
    }

    fn get_name() -> &'static str
    where
        Self: Sized,
    {
        "Breathing Single"
    }

    fn get_varargs(&mut self) -> &[u8] {
        &self.args
    }

    fn clone_box(&self) -> Box<dyn Effect> {
        Box::new(*self)
    }

    fn save(&mut self) -> EffectSave {
        EffectSave {
            args: self.args.to_vec(),
            name: String::from("Breathing Single"),
        }
    }

    fn get_state(&mut self) -> Vec<u8> {
        self.kbd.get_curr_state()
    }
}

///
/// WHEEL KEYBOARD EFFECT
/// A rotating rainbow, computed from each key's angle around the keyboard's center - "Wave" but
/// circular instead of linear. Built entirely in software on the per-key custom-frame engine
/// (proven working via the paint canvas/Random All) rather than the hardware's native Wheel
/// effect, which OpenRazer's own device table only lists for BlackWidow V4-family keyboards and
/// which produced no visible output on this laptop across every direction/speed combination
/// tried.
///
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let h = h.rem_euclid(1.0) * 6.0;
    let i = h.floor();
    let f = h - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match i as i32 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

#[derive(Clone)]
pub struct Wheel {
    kbd: board::KeyboardData,
    args: Vec<u8>,
    phase: f32,
    direction: f32, // +1.0 or -1.0
    turns_per_sec: f32,
}

impl Effect for Wheel {
    fn new(args: Vec<u8>) -> Box<dyn Effect> {
        // args[0]: direction (1 or 2, matching the wire values already used elsewhere in this
        // project - 2 means reversed). args[1]: speed as a plain 0-100 percentage (this is our
        // own effect, not a hardware byte, so the GUI just sends one of 4 fixed presets).
        let direction = if *args.get(0).unwrap_or(&1) == 2 {
            -1.0
        } else {
            1.0
        };
        let speed_pct = (*args.get(1).unwrap_or(&50) as f32).clamp(0.0, 100.0);
        Box::new(Wheel {
            kbd: board::KeyboardData::new(),
            args,
            phase: 0.0,
            direction,
            turns_per_sec: 0.05 + (speed_pct / 100.0) * 0.75,
        })
    }

    fn update(&mut self) -> board::KeyboardData {
        let rows = board::ROWS as f32;
        let cols = board::cols() as f32;
        let cy = (rows - 1.0) / 2.0;
        let cx = (cols - 1.0) / 2.0;
        for row in 0..board::ROWS {
            for col in 0..board::cols() {
                let dy = row as f32 - cy;
                let dx = col as f32 - cx;
                let angle = dy.atan2(dx); // -pi..pi
                let hue = (angle / (2.0 * std::f32::consts::PI)) + self.phase;
                let (r, g, b) = hsv_to_rgb(hue, 1.0, 1.0);
                self.kbd.set_key_colour(row, col, r, g, b);
            }
        }
        self.phase += self.direction * self.turns_per_sec * (ANIMATION_SLEEP_MS as f32 / 1000.0);
        self.phase = self.phase.rem_euclid(1.0);
        self.kbd
    }

    fn get_name() -> &'static str
    where
        Self: Sized,
    {
        "Wheel"
    }

    fn get_varargs(&mut self) -> &[u8] {
        &self.args
    }

    fn clone_box(&self) -> Box<dyn Effect> {
        Box::new(self.clone())
    }

    fn save(&mut self) -> EffectSave {
        EffectSave {
            args: self.args.clone(),
            name: String::from("Wheel"),
        }
    }

    fn get_state(&mut self) -> Vec<u8> {
        self.kbd.get_curr_state()
    }
}

///
/// AUDIO METER KEYBOARD EFFECT
/// A per-frequency-band "firework" - each of the 16 columns is a different frequency band (bass
/// on the left, mids in the middle, treble on the right), driven by real onset/beat detection
/// (see daemon/audio.rs): a column only lights up when that band's energy genuinely jumps above
/// its own recent baseline (a real hit), shoots to its peak instantly, then falls back to black
/// unconditionally - not a continuous level meter that stays lit as long as the sound does.
/// Silence or a sustained, unchanging tone correctly produces a dark keyboard; only real
/// transients burst. The boundary row of each column is partially dimmed by the fractional part
/// of that column's current pulse height for a
/// smoother rise/fall given only 6 physical LED rows to work with. `Effect::new` (required by the
/// trait, used by save/restore) can't carry the shared Arc<Mutex<[f32; BANDS]>> that live audio
/// capture needs, so it builds a harmless silent instance instead - the real, live-audio-backed
/// construction path is `SoundBar::new_live`, used exclusively by daemon.rs's SetSoundBarEffect
/// handler. This effect is deliberately NOT wired into EffectLayer::from_save (kbd/mod.rs) - it
/// shouldn't silently resume spawning an audio-capture subprocess on daemon restart.
///
pub const COLOR_MODE_RAINBOW: u8 = 1;
pub const COLOR_MODE_STATIC: u8 = 2;
pub const COLOR_MODE_INTENSITY: u8 = 3;

/// Smooth 5-stop gradient driven by that column's own current intensity (not row position), so
/// the whole bar's color shifts as it gets louder: blue/purple (quiet) -> cyan/green ->
/// yellow/orange -> red -> white (peak), rather than a fixed color per row height.
fn intensity_gradient(t: f32) -> (u8, u8, u8) {
    const STOPS: [(f32, (f32, f32, f32)); 5] = [
        (0.0, (90.0, 0.0, 200.0)),
        (0.33, (0.0, 220.0, 160.0)),
        (0.66, (255.0, 160.0, 0.0)),
        (0.85, (255.0, 40.0, 40.0)),
        (1.0, (255.0, 255.0, 255.0)),
    ];
    let t = t.clamp(0.0, 1.0);
    for pair in STOPS.windows(2) {
        let (t0, c0) = pair[0];
        let (t1, c1) = pair[1];
        if t <= t1 {
            let f = if t1 > t0 { (t - t0) / (t1 - t0) } else { 0.0 };
            return (
                (c0.0 + (c1.0 - c0.0) * f) as u8,
                (c0.1 + (c1.1 - c0.1) * f) as u8,
                (c0.2 + (c1.2 - c0.2) * f) as u8,
            );
        }
    }
    (255, 255, 255)
}

fn dim(color: (u8, u8, u8), brightness: f32) -> (u8, u8, u8) {
    let b = brightness.clamp(0.0, 1.0);
    (
        (color.0 as f32 * b) as u8,
        (color.1 as f32 * b) as u8,
        (color.2 as f32 * b) as u8,
    )
}

pub struct SoundBar {
    kbd: board::KeyboardData,
    args: Vec<u8>,
    levels: std::sync::Arc<std::sync::Mutex<[f32; crate::audio::BANDS]>>,
}

impl SoundBar {
    /// Real construction path - `levels` is the live, continuously-updated shared array from an
    /// active `audio::AudioCapture`.
    pub fn new_live(
        args: Vec<u8>,
        levels: std::sync::Arc<std::sync::Mutex<[f32; crate::audio::BANDS]>>,
    ) -> Box<dyn Effect> {
        Box::new(SoundBar {
            kbd: board::KeyboardData::new(),
            args,
            levels,
        })
    }
}

impl Effect for SoundBar {
    fn new(args: Vec<u8>) -> Box<dyn Effect> {
        Box::new(SoundBar {
            kbd: board::KeyboardData::new(),
            args,
            levels: std::sync::Arc::new(std::sync::Mutex::new([0.0; crate::audio::BANDS])),
        })
    }

    fn update(&mut self) -> board::KeyboardData {
        let color_mode = *self.args.first().unwrap_or(&COLOR_MODE_RAINBOW);
        let (sr, sg, sb) = (
            *self.args.get(1).unwrap_or(&0),
            *self.args.get(2).unwrap_or(&255),
            *self.args.get(3).unwrap_or(&0),
        );
        // 0-100, caps the final output so the effect isn't forced to always run at full
        // brightness - configurable from the GUI.
        let brightness_limit = (*self.args.get(4).unwrap_or(&100) as f32 / 100.0).clamp(0.0, 1.0);
        let levels = self
            .levels
            .lock()
            .map(|l| *l)
            .unwrap_or([0.0; crate::audio::BANDS]);

        // One frequency band per column, stretched across wider grids.
        let cols = board::cols();
        for col in 0..cols {
            let level = levels[col * crate::audio::BANDS / cols];
            let exact = level * board::ROWS as f32;
            let full_rows = exact.floor() as usize;
            let partial_brightness = exact - full_rows as f32;

            let base_color = match color_mode {
                COLOR_MODE_STATIC => (sr, sg, sb),
                COLOR_MODE_INTENSITY => intensity_gradient(level),
                _ => hsv_to_rgb(col as f32 / cols as f32, 1.0, 1.0),
            };
            let base_color = dim(base_color, brightness_limit);

            for row in 0..board::ROWS {
                let row_from_bottom = board::ROWS - 1 - row;
                let (r, g, b) = if row_from_bottom < full_rows {
                    base_color
                } else if row_from_bottom == full_rows {
                    dim(base_color, partial_brightness)
                } else {
                    (0, 0, 0)
                };
                self.kbd.set_key_colour(row, col, r, g, b);
            }
        }
        self.kbd
    }

    fn get_name() -> &'static str
    where
        Self: Sized,
    {
        "Audio Meter"
    }

    fn get_varargs(&mut self) -> &[u8] {
        &self.args
    }

    fn clone_box(&self) -> Box<dyn Effect> {
        Box::new(SoundBar {
            kbd: self.kbd,
            args: self.args.clone(),
            levels: self.levels.clone(),
        })
    }

    fn save(&mut self) -> EffectSave {
        EffectSave {
            args: self.args.clone(),
            name: String::from("Audio Meter"),
        }
    }

    fn get_state(&mut self) -> Vec<u8> {
        self.kbd.get_curr_state()
    }
}

///
/// SOFTWARE BREATHING KEYBOARD EFFECT
/// A whole-keyboard smooth fade in/out, replacing the native hardware Breathing effect on
/// devices with per_key_rgb - the native protocol was verified byte-correct against OpenRazer's
/// reference (mode encoding, data_size, transaction ID all fixed and confirmed via live protocol
/// testing on this exact hardware) but never produced any visible output, with no further
/// diagnostic lead available without a real USB capture. Same wire params as before (mode
/// 1=single/2=dual/3=random, then 0/2 RGB triples) and same GUI/CLI surface - daemon.rs only
/// substitutes this backend for devices that have per_key_rgb; other hardware keeps the
/// original native path untouched, since its breathing may work fine there.
///
pub struct SoftBreathing {
    kbd: board::KeyboardData,
    args: Vec<u8>,
    phase: f32,
    cycle_count: u32,
    mode: u8,
    color1: (u8, u8, u8),
    color2: (u8, u8, u8),
    random_color: (u8, u8, u8),
}

fn random_color() -> (u8, u8, u8) {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (rng.r#gen(), rng.r#gen(), rng.r#gen())
}

impl Effect for SoftBreathing {
    fn new(args: Vec<u8>) -> Box<dyn Effect> {
        let mode = *args.first().unwrap_or(&1);
        let color1 = (
            *args.get(1).unwrap_or(&255),
            *args.get(2).unwrap_or(&255),
            *args.get(3).unwrap_or(&255),
        );
        let color2 = (
            *args.get(4).unwrap_or(&0),
            *args.get(5).unwrap_or(&0),
            *args.get(6).unwrap_or(&0),
        );
        Box::new(SoftBreathing {
            kbd: board::KeyboardData::new(),
            args,
            phase: 0.0,
            cycle_count: 0,
            mode,
            color1,
            color2,
            random_color: random_color(),
        })
    }

    fn update(&mut self) -> board::KeyboardData {
        // ~3 second full breathe cycle at the ~20 ticks/sec render rate.
        const TICKS_PER_CYCLE: f32 = 60.0;
        self.phase += 1.0 / TICKS_PER_CYCLE;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
            self.cycle_count = self.cycle_count.wrapping_add(1);
            if self.mode == 3 {
                self.random_color = random_color();
            }
        }
        // Triangle wave: 0 -> 1 -> 0 across the cycle, rather than a hard on/off.
        let brightness = 1.0 - (2.0 * self.phase - 1.0).abs();

        let target = match self.mode {
            // Dual: alternate whole pulses between the two colors rather than blending them,
            // matching how real breathing effects look (each color gets its own fade in/out).
            2 => {
                if self.cycle_count.is_multiple_of(2) {
                    self.color1
                } else {
                    self.color2
                }
            }
            3 => self.random_color,
            _ => self.color1,
        };
        let (r, g, b) = (
            (target.0 as f32 * brightness) as u8,
            (target.1 as f32 * brightness) as u8,
            (target.2 as f32 * brightness) as u8,
        );
        self.kbd.set_kbd_colour(r, g, b);
        self.kbd
    }

    fn get_name() -> &'static str
    where
        Self: Sized,
    {
        "Software Breathing"
    }

    fn get_varargs(&mut self) -> &[u8] {
        &self.args
    }

    fn clone_box(&self) -> Box<dyn Effect> {
        Box::new(SoftBreathing {
            kbd: self.kbd,
            args: self.args.clone(),
            phase: self.phase,
            cycle_count: self.cycle_count,
            mode: self.mode,
            color1: self.color1,
            color2: self.color2,
            random_color: self.random_color,
        })
    }

    fn save(&mut self) -> EffectSave {
        EffectSave {
            args: self.args.clone(),
            name: String::from("Software Breathing"),
        }
    }

    fn get_state(&mut self) -> Vec<u8> {
        self.kbd.get_curr_state()
    }
}

///
/// STARS KEYBOARD EFFECT
/// A timed version of the paint canvas's "Random All" button: every key gets a fresh
/// independent random color, re-rolled on a fixed interval instead of once per button press.
pub struct Stars {
    kbd: board::KeyboardData,
    args: Vec<u8>,
    ticks: u32,
    interval_ticks: u32,
}

fn randomize_all(kbd: &mut board::KeyboardData) {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    for row in 0..board::ROWS {
        for col in 0..board::cols() {
            kbd.set_key_colour(row, col, rng.r#gen(), rng.r#gen(), rng.r#gen());
        }
    }
}

impl Effect for Stars {
    fn new(args: Vec<u8>) -> Box<dyn Effect> {
        // Wire speed 1-4 (1=slowest, 4=fastest) maps to a re-randomize interval of
        // 1.00/0.75/0.50/0.25 seconds. update() is ticked at ANIMATION_FPS (~20/sec, see
        // mod.rs), so interval_ticks = interval_seconds * ANIMATION_FPS.
        let speed = (*args.first().unwrap_or(&1)).clamp(1, 4);
        let interval_ticks = match speed {
            1 => super::ANIMATION_FPS,
            2 => (super::ANIMATION_FPS * 3) / 4,
            3 => super::ANIMATION_FPS / 2,
            _ => super::ANIMATION_FPS / 4,
        } as u32;
        let mut kbd = board::KeyboardData::new();
        randomize_all(&mut kbd);
        Box::new(Stars {
            kbd,
            args,
            ticks: 0,
            interval_ticks,
        })
    }

    fn update(&mut self) -> board::KeyboardData {
        self.ticks += 1;
        if self.ticks >= self.interval_ticks {
            self.ticks = 0;
            randomize_all(&mut self.kbd);
        }
        self.kbd
    }

    fn get_name() -> &'static str
    where
        Self: Sized,
    {
        "Stars"
    }

    fn get_varargs(&mut self) -> &[u8] {
        &self.args
    }

    fn clone_box(&self) -> Box<dyn Effect> {
        Box::new(Stars {
            kbd: self.kbd,
            args: self.args.clone(),
            ticks: self.ticks,
            interval_ticks: self.interval_ticks,
        })
    }

    fn save(&mut self) -> EffectSave {
        EffectSave {
            args: self.args.clone(),
            name: String::from("Stars"),
        }
    }

    fn get_state(&mut self) -> Vec<u8> {
        self.kbd.get_curr_state()
    }
}

///
/// RIPPLE KEYBOARD EFFECT
/// Each key press sends a ring of light outward from that key across the keyboard, on black -
/// the same software effect OpenRazer implements for Razer keyboards (their firmware has no
/// native ripple). Args: [color_mode, r, g, b, speed]. RIPPLE_STATIC draws every ring in
/// (r, g, b); rainbow (1) colors the ring by its distance from the key, so it sweeps through
/// Wheel's hue order as it travels; RIPPLE_RANDOM gives each press its own random color (as in
/// OpenRazer's random-colour ripple). Speed 1-4 sets how fast rings expand.
///
pub struct Ripple {
    kbd: board::KeyboardData,
    args: Vec<u8>,
    presses: crate::keys::Presses,
    ripples: Vec<Ring>,
}

#[derive(Clone)]
struct Ring {
    row: usize,
    col: usize,
    /// Frames since the key press.
    age: u32,
    /// Ring color, or None to color by distance (rainbow).
    color: Option<(u8, u8, u8)>,
}

/// Ripple color modes; any other value (1) is rainbow.
pub const RIPPLE_STATIC: u8 = 2;
pub const RIPPLE_RANDOM: u8 = 3;
/// Width of the lit ring, in keys - matches OpenRazer's ripple.
const RIPPLE_WIDTH: f32 = 2.0;
/// Distance over which a rainbow ring goes once around the color wheel (a 16-column keyboard's
/// width), kept fixed so wider keyboards show the same gradient.
const RIPPLE_RAINBOW_SPAN: f32 = 16.0;
/// Oldest rings are dropped beyond this many, to bound per-frame work during fast typing.
const RIPPLE_MAX_ACTIVE: usize = 24;

impl Ripple {
    pub fn new_live(args: Vec<u8>, presses: crate::keys::Presses) -> Box<dyn Effect> {
        Box::new(Ripple {
            kbd: board::KeyboardData::new(),
            args,
            presses,
            ripples: Vec::new(),
        })
    }

    /// Ring expansion in keys per second; 3 is OpenRazer's default of 24.
    fn keys_per_second(&self) -> f32 {
        match self.args.get(4).copied().unwrap_or(3) {
            1 => 12.0,
            2 => 18.0,
            3 => 24.0,
            _ => 32.0,
        }
    }

    fn ring_color(&self) -> Option<(u8, u8, u8)> {
        match self.args.first().copied() {
            Some(RIPPLE_STATIC) => Some((
                self.args.get(1).copied().unwrap_or(0),
                self.args.get(2).copied().unwrap_or(255),
                self.args.get(3).copied().unwrap_or(0),
            )),
            Some(RIPPLE_RANDOM) => Some(random_color()),
            _ => None,
        }
    }
}

impl Effect for Ripple {
    fn new(args: Vec<u8>) -> Box<dyn Effect> {
        Ripple::new_live(args, Default::default())
    }

    fn update(&mut self) -> board::KeyboardData {
        let pressed: Vec<(usize, usize)> = self
            .presses
            .lock()
            .map(|mut p| p.drain(..).collect())
            .unwrap_or_default();
        for (row, col) in pressed {
            let color = self.ring_color();
            self.ripples.push(Ring {
                row,
                col,
                age: 0,
                color,
            });
        }
        let excess = self.ripples.len().saturating_sub(RIPPLE_MAX_ACTIVE);
        self.ripples.drain(..excess);

        let keys_per_frame = self.keys_per_second() / super::ANIMATION_FPS as f32;
        let cols = board::cols();
        // Corner to corner: once a ring's trailing edge passes this, it's off the keyboard.
        let max_dist = ((board::ROWS - 1) as f32).hypot((cols - 1) as f32);

        for row in 0..board::ROWS {
            for col in 0..cols {
                let mut out = (0u8, 0u8, 0u8);
                for ring in &self.ripples {
                    let radius = ring.age as f32 * keys_per_frame;
                    let dist = (row as f32 - ring.row as f32).hypot(col as f32 - ring.col as f32);
                    // Brightest at the leading edge, fading across the ring's width behind it.
                    let behind = radius - dist;
                    if !(0.0..RIPPLE_WIDTH).contains(&behind) {
                        continue;
                    }
                    let base = ring
                        .color
                        .unwrap_or_else(|| hsv_to_rgb(dist / RIPPLE_RAINBOW_SPAN, 1.0, 1.0));
                    let (r, g, b) = dim(base, 1.0 - behind / RIPPLE_WIDTH);
                    out = (out.0.max(r), out.1.max(g), out.2.max(b));
                }
                self.kbd.set_key_colour(row, col, out.0, out.1, out.2);
            }
        }

        for ring in &mut self.ripples {
            ring.age += 1;
        }
        self.ripples
            .retain(|ring| ring.age as f32 * keys_per_frame < max_dist + RIPPLE_WIDTH);
        self.kbd
    }

    fn get_name() -> &'static str
    where
        Self: Sized,
    {
        "Ripple"
    }

    fn get_varargs(&mut self) -> &[u8] {
        &self.args
    }

    fn clone_box(&self) -> Box<dyn Effect> {
        Box::new(Ripple {
            kbd: self.kbd,
            args: self.args.clone(),
            presses: self.presses.clone(),
            ripples: self.ripples.clone(),
        })
    }

    fn save(&mut self) -> EffectSave {
        EffectSave {
            args: self.args.clone(),
            name: String::from("Ripple"),
        }
    }

    fn get_state(&mut self) -> Vec<u8> {
        self.kbd.get_curr_state()
    }
}

///
/// CPU TEMPERATURE KEYBOARD EFFECT
/// The whole keyboard shows the CPU temperature as a color: blue at or below `cool` °C, through
/// cyan, green and yellow, to red at or above `hot` °C (the idea of OpenRazer's
/// cpu_temperature.py example). Args: [cool, hot]. The sensor is read once a second and the color
/// eases toward it, so short spikes don't flash the keyboard.
///
pub struct CpuTemperature {
    kbd: board::KeyboardData,
    args: Vec<u8>,
    ticks: u32,
    /// Smoothed temperature being displayed, and the latest reading it eases toward.
    shown: Option<f32>,
    target: Option<f32>,
}

/// Hue for fully cool (blue); fully hot is 0 (red).
const TEMP_COOL_HUE: f32 = 0.66;
/// Fraction of the remaining gap closed each frame (~1 s to settle at 20 FPS).
const TEMP_EASING: f32 = 0.15;

impl CpuTemperature {
    fn range(&self) -> (f32, f32) {
        let cool = self.args.first().copied().unwrap_or(45) as f32;
        let hot = (self.args.get(1).copied().unwrap_or(90) as f32).max(cool + 1.0);
        (cool, hot)
    }
}

impl Effect for CpuTemperature {
    fn new(args: Vec<u8>) -> Box<dyn Effect> {
        Box::new(CpuTemperature {
            kbd: board::KeyboardData::new(),
            args,
            ticks: 0,
            shown: None,
            target: None,
        })
    }

    fn update(&mut self) -> board::KeyboardData {
        if self.ticks == 0 {
            self.target = service::cpu_temperature().map(|t| t as f32).or(self.target);
        }
        self.ticks = (self.ticks + 1) % super::ANIMATION_FPS as u32;

        let Some(target) = self.target else {
            // No sensor: stay dark rather than show a meaningless color.
            self.kbd.set_kbd_colour(0, 0, 0);
            return self.kbd;
        };
        let shown = self
            .shown
            .map_or(target, |s| s + (target - s) * TEMP_EASING);
        self.shown = Some(shown);

        let (cool, hot) = self.range();
        let heat = ((shown - cool) / (hot - cool)).clamp(0.0, 1.0);
        let (r, g, b) = hsv_to_rgb(TEMP_COOL_HUE * (1.0 - heat), 1.0, 1.0);
        self.kbd.set_kbd_colour(r, g, b);
        self.kbd
    }

    fn get_name() -> &'static str
    where
        Self: Sized,
    {
        "CPU Temperature"
    }

    fn get_varargs(&mut self) -> &[u8] {
        &self.args
    }

    fn clone_box(&self) -> Box<dyn Effect> {
        Box::new(CpuTemperature {
            kbd: self.kbd,
            args: self.args.clone(),
            ticks: self.ticks,
            shown: self.shown,
            target: self.target,
        })
    }

    fn save(&mut self) -> EffectSave {
        EffectSave {
            args: self.args.clone(),
            name: String::from("CPU Temperature"),
        }
    }

    fn get_state(&mut self) -> Vec<u8> {
        self.kbd.get_curr_state()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(kbd: &mut board::KeyboardData, row: usize, col: usize) -> (u8, u8, u8) {
        let k = kbd.get_key_at(row * board::MAX_COLS + col);
        (k.red, k.green, k.blue)
    }

    #[test]
    fn temperature_maps_cool_to_blue_and_hot_to_red() {
        let mut fx = CpuTemperature {
            kbd: board::KeyboardData::new(),
            args: vec![45, 90],
            ticks: 1, // skip the sensor read
            shown: None,
            target: Some(40.0),
        };
        let (r, _, b) = key(&mut fx.update(), 0, 0);
        assert!(b > 200 && r < 20, "cool should be blue");

        fx.shown = None;
        fx.target = Some(95.0);
        let (r, g, b) = key(&mut fx.update(), 0, 0);
        assert!(r > 200 && g < 20 && b < 20, "hot should be red");
    }

    #[test]
    fn ripple_rings_start_at_the_pressed_key_and_expire() {
        let presses: crate::keys::Presses = Default::default();
        let mut fx = Ripple::new_live(vec![RIPPLE_STATIC, 0, 255, 0, 4], presses.clone());
        presses.lock().unwrap().push((3, 5));
        let mut frame = fx.update();
        assert_eq!(key(&mut frame, 3, 5), (0, 255, 0));
        assert_eq!(key(&mut frame, 0, 15), (0, 0, 0));
        // Fastest speed crosses a 6x16 keyboard in well under 2 s.
        for _ in 0..40 {
            frame = fx.update();
        }
        assert_eq!(key(&mut frame, 3, 5), (0, 0, 0));
    }

    #[test]
    fn random_ripples_keep_one_color_per_press() {
        let presses: crate::keys::Presses = Default::default();
        let mut fx = Ripple {
            kbd: board::KeyboardData::new(),
            args: vec![RIPPLE_RANDOM, 0, 0, 0, 1],
            presses: presses.clone(),
            ripples: Vec::new(),
        };
        presses.lock().unwrap().extend([(2, 8), (4, 3)]);
        fx.update();
        let colors: Vec<_> = fx.ripples.iter().map(|r| r.color).collect();
        assert!(colors.iter().all(Option::is_some));
        fx.update();
        assert_eq!(
            colors,
            fx.ripples.iter().map(|r| r.color).collect::<Vec<_>>()
        );
    }
}
