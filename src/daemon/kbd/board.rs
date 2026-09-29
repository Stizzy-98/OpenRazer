/*
use crate::driver_sysfs;
*/
use crate::device;
use std::cmp::Ordering;
use std::ops;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

// -- RGB Key channel --

// Every supported model's grid has 6 rows; the column count varies by model (16 on most Blades,
// up to 25 on the Blade Pro 2017 - from OpenRazer's MATRIX_DIMS). Frames are stored at the widest
// size, and only the detected model's columns are rendered and sent (see `cols`).
pub const ROWS: usize = service::MATRIX_ROWS;
pub const MAX_COLS: usize = service::MAX_MATRIX_COLS;

static COLS: AtomicUsize = AtomicUsize::new(16);

/// Columns in the detected model's per-key grid.
pub fn cols() -> usize {
    COLS.load(AtomicOrdering::Relaxed)
}

/// Set once at startup from the detected model's device-list entry.
pub fn set_cols(cols: usize) {
    COLS.store(cols.clamp(1, MAX_COLS), AtomicOrdering::Relaxed);
}

#[derive(Copy, Clone, Debug)]
/// Represents the colour channels for a key
pub struct KeyColour {
    /// Red channel
    pub red: u8,
    /// Green channel
    pub green: u8,
    /// Blue channel
    pub blue: u8,
}

/// Same as `KeyColour`, but uses f32 values, for more accurate frame by frame
/// colour blending in animations
#[derive(Copy, Clone, Debug)]
pub struct AnimatorKeyColour {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
}

impl AnimatorKeyColour {
    pub fn new_f(red: f32, green: f32, blue: f32) -> AnimatorKeyColour {
        AnimatorKeyColour { red, green, blue }
    }

    pub fn new_u(red: u8, green: u8, blue: u8) -> AnimatorKeyColour {
        AnimatorKeyColour {
            red: red as f32,
            green: green as f32,
            blue: blue as f32,
        }
    }

    /// Clamps a f32 between 0 and 255, returns a `u8`
    fn clamp_colour(inp: f32) -> u8 {
        let mut input = inp;
        if input > 255.0 {
            input = 255.0
        };
        if input < 0.0 {
            input = 0.0
        };
        input as u8
    }

    pub fn divide(&mut self, divisor: f32) -> AnimatorKeyColour {
        AnimatorKeyColour {
            red: self.red / divisor,
            green: self.green / divisor,
            blue: self.blue / divisor,
        }
    }

    pub fn get_clamped_colour(&self) -> KeyColour {
        KeyColour {
            red: AnimatorKeyColour::clamp_colour(self.red),
            green: AnimatorKeyColour::clamp_colour(self.green),
            blue: AnimatorKeyColour::clamp_colour(self.blue),
        }
    }
}

impl ops::Add for AnimatorKeyColour {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self {
            red: self.red + rhs.red,
            green: self.green + rhs.green,
            blue: self.blue + rhs.blue,
        }
    }
}

impl ops::Sub for AnimatorKeyColour {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self {
            red: self.red - rhs.red,
            green: self.green - rhs.green,
            blue: self.blue - rhs.blue,
        }
    }
}

impl ops::AddAssign for AnimatorKeyColour {
    fn add_assign(&mut self, rhs: AnimatorKeyColour) {
        self.red += rhs.red;
        self.green += rhs.green;
        self.blue += rhs.blue;
    }
}

impl ops::SubAssign for AnimatorKeyColour {
    fn sub_assign(&mut self, rhs: AnimatorKeyColour) {
        self.red -= rhs.red;
        self.green -= rhs.green;
        self.blue -= rhs.blue;
    }
}

impl PartialEq for AnimatorKeyColour {
    fn eq(&self, other: &AnimatorKeyColour) -> bool {
        self.red == other.red && self.blue == other.blue && self.green == other.green
    }
}

impl PartialOrd for AnimatorKeyColour {
    fn partial_cmp(&self, other: &AnimatorKeyColour) -> Option<Ordering> {
        if self.red == other.red && self.blue == other.blue && self.green == other.green {
            return Some(Ordering::Equal);
        } else if self.red >= other.red && self.blue >= other.blue && self.green >= other.green {
            return Some(Ordering::Greater);
        } else if self.red <= other.red && self.blue <= other.blue && self.green <= other.green {
            return Some(Ordering::Less);
        }
        None
    }
}

#[derive(Copy, Clone, Debug)]
/// Represents a horizontal row of keys on the keyboard
pub struct RowData {
    keys: [KeyColour; MAX_COLS],
}

impl RowData {
    /// Generates an empty keyboard row, with each key being white (FF,FF,FF)
    pub fn new() -> RowData {
        RowData {
            keys: [KeyColour {
                red: 255,
                green: 255,
                blue: 255,
            }; MAX_COLS],
        }
    }

    /// Sets key colour within the row
    ///
    /// # Parameters
    /// * pos - Key number within the matrix, starting from left side of the keyboard
    /// * r - Red channel value
    /// * g - Green channel value
    /// * b - Blue channel value
    pub fn set_key_color(&mut self, pos: usize, r: u8, g: u8, b: u8) {
        self.keys[pos] = KeyColour {
            red: r,
            green: g,
            blue: b,
        }
    }

    /// Sets the entire key row to a colour
    ///
    /// # Parameters
    /// * r - Red channel value
    /// * g - Green channel value
    /// * b - Blue channel value
    pub fn set_row_color(&mut self, r: u8, g: u8, b: u8) {
        (0..cols()).for_each(|x| self.set_key_color(x, r, g, b)) // Sets the entire row
    }

    pub fn get_row_data(&mut self) -> Vec<u8> {
        // *3 as itll be the RGB values
        let mut v = Vec::<u8>::with_capacity(3 * cols());
        self.keys[..cols()].iter().for_each(|k| {
            v.push(k.red);
            v.push(k.green);
            v.push(k.blue);
        });
        v
    }
}

#[derive(Copy, Clone, Debug)]
pub struct KeyboardData {
    rows: [RowData; ROWS],
    // brightness: u8,
}

impl KeyboardData {
    pub fn new() -> KeyboardData {
        KeyboardData {
            rows: [RowData::new(); ROWS],
        }
    }

    pub fn update_kbd(&mut self, laptop: &mut device::RazerLaptop) -> bool {
        for idx in 0..ROWS {
            laptop.set_custom_frame_data(idx as u8, self.rows[idx].get_row_data());
        }
        true
    }

    pub fn update_custom_mode(&mut self, laptop: &mut device::RazerLaptop) -> bool {
        laptop.set_custom_frame()
    }

    /// Sets a specific key in the keyboard matrix to a colour
    pub fn set_key_colour(&mut self, row: usize, col: usize, r: u8, g: u8, b: u8) {
        if row >= ROWS {
            return;
        }
        if col >= cols() {
            return;
        }
        self.rows[row].set_key_color(col, r, g, b)
    }

    /// Sets a vertical column on the keyboard to a colour
    pub fn set_col_colour(&mut self, col: usize, r: u8, g: u8, b: u8) {
        if col >= cols() {
            return;
        }
        for row_id in 0..ROWS {
            self.rows[row_id].set_key_color(col, r, g, b)
        }
    }

    /// Sets the entire keyboard to a colour
    pub fn set_kbd_colour(&mut self, r: u8, g: u8, b: u8) {
        for row_id in 0..ROWS {
            self.rows[row_id].set_row_color(r, g, b)
        }
    }

    /// Returns a specific key. `index` is `row * MAX_COLS + col` (the layer-mask layout).
    pub fn get_key_at(self, index: usize) -> KeyColour {
        self.rows[index / MAX_COLS].keys[index % MAX_COLS]
    }

    /// Internal function used only for the combining of effect layers
    pub fn set_key_at(&mut self, index: usize, col: KeyColour) {
        self.rows[index / MAX_COLS].keys[index % MAX_COLS] = col
    }

    /// The visible frame, row by row, `cols()` keys per row.
    pub fn get_curr_state(&mut self) -> Vec<u8> {
        let mut all_vals = Vec::<u8>::with_capacity(3 * cols() * ROWS);
        for row in self.rows.iter_mut() {
            all_vals.extend(&row.get_row_data());
        }
        all_vals
    }
}
