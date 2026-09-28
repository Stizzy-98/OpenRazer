pub mod board;
pub mod effects;
use crate::device;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

// Was hardcoded to 90 (15 * 6) everywhere below before board.rs's KEYS_PER_ROW fix (15 -> 16,
// see board.rs) - never actually exercised until now since the paint canvas bypasses this
// mask/layer system entirely, but Wheel (an animated effect) is the first real user of it.
pub const TOTAL_KEYS: usize = board::ROWS * board::KEYS_PER_ROW;

// Was 10 (100ms/tick) - too slow for Sound Bar to look like a live meter rather than discrete
// jumps. Doubling to 20 (50ms/tick) roughly doubles USB HID traffic for whichever per-key
// animated effect is active (Wheel, Sound Bar) - the 7 native whole-keyboard hardware effects
// don't use this loop at all, so this doesn't affect them.
const ANIMATION_FPS: u64 = 20;

pub const ANIMATION_SLEEP_MS: u64 = (1000.0 / ANIMATION_FPS as f32) as u64;

pub fn get_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[derive(Serialize, Deserialize)]
pub struct EffectSave {
    args: Vec<u8>,
    name: String,
}

/// Base effect trait.
/// An effect is a lighting function that is updated 30 times per second
/// in order to create an animation of some description on the laptop's
/// keyboard
#[allow(dead_code, clippy::new_ret_no_self)]
pub trait Effect: Send + Sync {
    /// Returns a new instance of an Effect
    fn new(args: Vec<u8>) -> Box<dyn Effect>
    where
        Self: Sized;
    /// Updates the keyboard, returning the current state of the keyboard
    /// Called 30 times per second by the Effect Manager
    fn update(&mut self) -> board::KeyboardData;
    /// Returns the arguments used to spawn the effect
    fn get_varargs(&mut self) -> &[u8];
    /// Returns the name of the effect (Unique identifier)
    fn get_name() -> &'static str
    where
        Self: Sized;
    fn clone_box(&self) -> Box<dyn Effect>;
    fn save(&mut self) -> EffectSave;
    fn get_state(&mut self) -> Vec<u8>;
}

/// An effect combined with a mask layer.
/// The mask layer tells the Effect Manager to apply the given
/// Effect to. This allows for stacked effects
struct EffectLayer {
    /// Mask for keys
    key_mask: Vec<bool>,
    effect: Box<dyn Effect>,
}

impl EffectLayer {
    fn new(effect: Box<dyn Effect>, mask: [bool; TOTAL_KEYS]) -> EffectLayer {
        EffectLayer {
            key_mask: mask.to_vec(),
            effect,
        }
    }

    fn update(&mut self) -> board::KeyboardData {
        self.effect.update()
    }

    fn get_save(&mut self) -> Option<serde_json::Value> {
        match serde_json::to_value(self.effect.save()) {
            Ok(mut x) => {
                let keys = match serde_json::to_value(&self.key_mask) {
                    Ok(k) => k,
                    Err(e) => {
                        eprintln!("Failed to serialize key_mask: {}", e);
                        return None;
                    }
                };
                match x.as_object_mut() {
                    Some(obj) => {
                        obj.insert(String::from("key_mask"), keys);
                    }
                    None => {
                        eprintln!("Effect save is not a JSON object");
                        return None;
                    }
                }
                Some(x)
            }
            Err(_) => None,
        }
    }

    fn from_save(json: serde_json::Value) -> Option<EffectLayer> {
        if json["key_mask"].is_null() || json["name"].is_null() || json["args"].is_null() {
            eprintln!("Missing data for effect!");
            return None;
        }
        let key_mask: Vec<bool> = match serde_json::from_value(json["key_mask"].clone()) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Failed to deserialize key_mask: {}", e);
                return None;
            }
        };
        if key_mask.len() != TOTAL_KEYS {
            eprintln!(
                "Invalid key count effect. Expected {}, found {}",
                TOTAL_KEYS,
                key_mask.len()
            );
            return None;
        }
        let name: String = match serde_json::from_value(json["name"].clone()) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Failed to deserialize effect name: {}", e);
                return None;
            }
        };
        let args: Vec<u8> = match serde_json::from_value(json["args"].clone()) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Failed to deserialize effect args: {}", e);
                return None;
            }
        };

        let effect: Option<Box<dyn Effect>> = match name.as_str() {
            "Static" => Some(effects::Static::new(args)),
            "Wave Gradient" => Some(effects::WaveGradient::new(args)),
            "Breathing Single" => Some(effects::BreathSingle::new(args)),
            "Wheel" => Some(effects::Wheel::new(args)),
            "Static Gradient" => Some(effects::StaticGradient::new(args)),
            "Software Breathing" => Some(effects::SoftBreathing::new(args)),
            "Stars" => Some(effects::Stars::new(args)),
            // Needs the daemon's live audio capture to construct - restored by the daemon's
            // startup path instead (see `restore_audio_meter` in daemon.rs).
            "Audio Meter" => return None,
            // Same for Ripple, which needs the live key capture (see `restore_ripple`).
            "Ripple" => return None,
            _ => None,
        };
        match effect {
            Some(e) => Some(EffectLayer {
                key_mask,
                effect: e,
            }),
            None => {
                eprintln!("Effect failed to load. Invalid name: {}", name);
                None
            }
        }
    }

    pub fn get_state(&mut self) -> Vec<u8> {
        self.effect.get_state()
    }
}
pub struct EffectManager {
    layers: Vec<EffectLayer>,
    last_update_ms: u128,
    render_board: board::KeyboardData,
}

impl EffectManager {
    pub fn new() -> EffectManager {
        EffectManager {
            layers: vec![],
            last_update_ms: get_millis(),
            render_board: board::KeyboardData::new(),
        }
    }

    pub fn push_effect(&mut self, effect: Box<dyn Effect>, mask: [bool; TOTAL_KEYS]) {
        self.layers.push(EffectLayer::new(effect, mask))
    }

    pub fn pop_effect(&mut self, laptop: &mut device::RazerLaptop) {
        // Only reset to black/custom-frame-mode if a software layer actually existed - this used
        // to run unconditionally (is_empty() is trivially true when popping an already-empty
        // stack, which is the common case for every native hardware effect call). That sent a
        // spurious "enter custom-frame mode, show black" HID sequence immediately before every
        // single native effect command (Static/Off/Wave/Reactive/Spectrum/Starlight), even when
        // no software layer was running - two back-to-back mode-switch writes with nothing to
        // verify the first settled before the second one (the real command) went out, which is
        // what was actually causing native Static colour clicks to intermittently stick on a
        // stale/wrong colour indefinitely instead of applying the newly selected one.
        let had_layer = self.layers.pop().is_some();
        if had_layer && self.layers.is_empty() {
            self.render_board.set_kbd_colour(0, 0, 0);
            self.render_board.update_kbd(laptop);
            self.render_board.update_custom_mode(laptop);
        }
    }

    pub fn update(&mut self, laptop: &mut device::RazerLaptop) {
        // Do nothing if we have no effects!
        if self.layers.is_empty() {
            return;
        }
        for layer in self.layers.iter_mut() {
            let tmp_board = layer.update();
            for (pos, state) in layer.key_mask.iter().enumerate() {
                if *state {
                    self.render_board.set_key_at(pos, tmp_board.get_key_at(pos))
                }
            }
        }
        // Don't forget to actually render the board
        self.last_update_ms = get_millis();
        self.render_board.update_kbd(laptop);
        self.render_board.update_custom_mode(laptop);
    }

    pub fn save(&mut self) -> serde_json::value::Value {
        let mut save_json = json!({"effects" : []});

        let tmp_saves: Vec<Option<serde_json::Value>> =
            self.layers.iter_mut().map(|l| l.get_save()).collect();

        if let Some(arr) = save_json["effects"].as_array_mut() {
            for save in tmp_saves {
                if let Some(x) = save {
                    arr.push(x);
                } else {
                    eprintln!("Warning, discarding effect!");
                }
            }
        }
        save_json
    }

    pub fn load_from_save(&mut self, mut json: serde_json::Value) {
        if json["effects"].is_null() {
            eprintln!("Invalid json. No effects field!");
            return;
        }
        if let Some(effects) = json["effects"].as_array_mut() {
            for e in effects {
                if let Some(x) = EffectLayer::from_save(e.clone()) {
                    self.layers.push(x);
                } else {
                    eprintln!("Error adding effect");
                }
            }
        } else {
            eprintln!("Effects field is not an array!");
        }
    }

    /// Paints a single key directly, bypassing the animated Effect/layer system entirely.
    /// Clears any active animated layers first so nothing overwrites the painted frame on the
    /// next animator tick (`update()` already no-ops whenever `layers` is empty).
    pub fn set_custom_key(
        &mut self,
        index: usize,
        r: u8,
        g: u8,
        b: u8,
        laptop: &mut device::RazerLaptop,
    ) -> bool {
        // Client-supplied (0-255); anything past the 6x16 grid would index out of bounds and
        // panic the daemon.
        if index >= TOTAL_KEYS {
            return false;
        }
        self.layers.clear();
        self.render_board.set_key_at(
            index,
            board::KeyColour {
                red: r,
                green: g,
                blue: b,
            },
        );
        self.render_board.update_kbd(laptop);
        self.render_board.update_custom_mode(laptop);
        true
    }

    /// Fills every key directly, same bypass as `set_custom_key`.
    pub fn fill_custom(&mut self, r: u8, g: u8, b: u8, laptop: &mut device::RazerLaptop) {
        self.layers.clear();
        self.render_board.set_kbd_colour(r, g, b);
        self.render_board.update_kbd(laptop);
        self.render_board.update_custom_mode(laptop);
    }

    /// Fills every key with an independent random color. Uses a small inline xorshift32 PRNG
    /// (seeded from the current time) instead of adding a `rand` dependency for one button.
    pub fn randomize_custom(&mut self, laptop: &mut device::RazerLaptop) {
        self.layers.clear();
        let mut seed = (get_millis() as u32)
            .wrapping_mul(2654435761)
            .wrapping_add(1);
        let mut next_byte = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed & 0xFF) as u8
        };
        for row in 0..board::ROWS {
            for col in 0..board::KEYS_PER_ROW {
                let (r, g, b) = (next_byte(), next_byte(), next_byte());
                self.render_board.set_key_colour(row, col, r, g, b);
            }
        }
        self.render_board.update_kbd(laptop);
        self.render_board.update_custom_mode(laptop);
    }

    pub fn get_map(&mut self, layer_id: i32) -> Vec<u8> {
        if layer_id < 0 {
            // Requesting global layer
            return self.render_board.get_curr_state();
        }
        let idx = layer_id as usize;
        if idx < self.layers.len() {
            return self.layers[idx].get_state();
        }
        eprintln!("Invalid layer id: {}", layer_id);
        vec![]
    }
}
