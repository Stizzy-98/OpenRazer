use clap::{CommandFactory, Parser, Subcommand, ValueEnum, error::ErrorKind};
use service::{SupportedDevice, comms, keyboard_layout_name, valid_bho_threshold};

// Output of the `read` commands is parsed by the KDE widget: it greps the first number of
// `read power` and the last number of `read fan`, `brightness`, and `logo`, and looks for "on"/"off"
// in `read bho`. Keep those numbers in the output and in that order.

#[derive(Parser)]
#[command(
    version,
    about = "Razer laptop control from the command line",
    name = "razer-cli"
)]
struct Cli {
    #[command(subcommand)]
    args: Args,
}

#[derive(Subcommand)]
enum Args {
    /// Read the current configuration of the device for some attribute
    Read {
        #[command(subcommand)]
        attr: ReadAttr,
    },
    /// Write a new configuration to the device for some attribute
    Write {
        #[command(subcommand)]
        attr: WriteAttr,
    },
    /// Keyboard effects built into the firmware
    StandardEffect {
        #[command(subcommand)]
        effect: StandardEffect,
    },
    /// Older per-key effects (static, gradients, breathing)
    Effect {
        #[command(subcommand)]
        effect: Effect,
    },
    /// Wheel: a rainbow rotating around the keyboard
    Wheel(WheelParams),
    /// Audio Meter: a live spectrum of what's playing
    AudioMeter(AudioMeterParams),
    /// Stars: every key re-rolls to a random color on an interval
    Stars(StarsParams),
    /// Ripple: rings of light spreading out from each key press
    Ripple(RippleParams),
    /// CPU Temperature: the whole keyboard colored by CPU temperature
    Temperature(TemperatureParams),
    /// Paint individual keys or the whole keyboard
    Paint {
        #[command(subcommand)]
        action: Paint,
    },
}

#[derive(Parser)]
struct WheelParams {
    /// direction (1 or 2)
    #[arg(value_parser = clap::value_parser!(u8).range(1..=2))]
    direction: u8,
    /// speed percentage (the app's steps 1-4 are 25, 50, 75, 100)
    #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
    speed: u8,
}

#[derive(Parser)]
struct AudioMeterParams {
    /// color mode (1=rainbow, 2=static, 3=intensity gradient)
    #[arg(value_parser = clap::value_parser!(u8).range(1..=3))]
    color_mode: u8,
    /// red (0-255, used when color_mode=2)
    red: u8,
    /// green (0-255, used when color_mode=2)
    green: u8,
    /// blue (0-255, used when color_mode=2)
    blue: u8,
    /// sensitivity percentage (1-200, 100 = normal)
    #[arg(value_parser = clap::value_parser!(u8).range(1..=200))]
    sensitivity: u8,
    /// decay percentage (0-100, low is snappy, high lingers longer)
    #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
    decay: u8,
    /// brightness limit percentage (0-100)
    #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
    brightness: u8,
}

#[derive(Parser)]
struct StarsParams {
    /// speed (1-4): 1=slowest (re-rolls every 1.00s), 4=fastest (every 0.25s)
    #[arg(value_parser = clap::value_parser!(u8).range(1..=4))]
    speed: u8,
}

#[derive(Parser)]
struct RippleParams {
    /// color mode (1=rainbow, 2=static, 3=random color per press)
    #[arg(value_parser = clap::value_parser!(u8).range(1..=3))]
    color_mode: u8,
    /// red (0-255, used when color_mode=2)
    red: u8,
    /// green (0-255, used when color_mode=2)
    green: u8,
    /// blue (0-255, used when color_mode=2)
    blue: u8,
    /// speed (1-4): 1=slowest, 4=fastest
    #[arg(value_parser = clap::value_parser!(u8).range(1..=4))]
    speed: u8,
}

#[derive(Parser)]
struct TemperatureParams {
    /// °C at or below which the keyboard is fully blue
    cool: u8,
    /// °C at or above which the keyboard is fully red
    hot: u8,
}

#[derive(Subcommand)]
enum Paint {
    /// Paint one key (row and column on the lighting grid, from 0)
    Key(PaintKeyParams),
    /// Fill every key with one color
    Fill(StaticParams),
    /// Give every key its own random color
    Random,
    /// Turn every key off
    Clear,
}

#[derive(Parser)]
struct PaintKeyParams {
    /// row (0-5, top to bottom)
    row: u8,
    /// column (0 to the grid width - 1, left to right)
    col: u8,
    /// red (0-255)
    red: u8,
    /// green (0-255)
    green: u8,
    /// blue (0-255)
    blue: u8,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum OnOff {
    On,
    Off,
}

impl OnOff {
    pub fn is_on(&self) -> bool {
        matches!(self, Self::On)
    }
}

#[derive(Subcommand)]
enum ReadAttr {
    /// Read the current fan speed
    Fan(AcStateParam),
    /// Read the current power mode
    Power(AcStateParam),
    /// Read the current brightness
    Brightness(AcStateParam),
    /// Read the current logo mode
    Logo(AcStateParam),
    /// Read the keyboard lights' idle timeout
    Idle(AcStateParam),
    /// Read the current sync mode
    Sync,
    /// Read the current bho mode
    Bho,
    /// Read actual fan RPM from hardware
    FanRpm,
    /// Read GPU status information
    Gpu,
    /// Read the model, lighting grid, keyboard firmware, and keyboard layout
    Device,
    /// Read the lighting effect that's currently showing
    Effect,
    /// Read the color of every key, one grid row per line
    Frame,
}

#[derive(Subcommand)]
enum WriteAttr {
    /// Set the fan speed
    Fan(FanParams),
    /// Set the power mode
    Power(PowerParams),
    /// Set the brightness of the keyboard
    Brightness(BrightnessParams),
    /// Set the logo mode
    Logo(LogoParams),
    /// Turn the keyboard lights off after this many idle minutes (GNOME only)
    Idle(IdleParams),
    /// Set sync
    Sync(SyncParams),
    /// Set battery health optimization
    Bho(BhoParams),
    /// Set dGPU runtime power management
    RuntimePm(RuntimePmParams),
    /// Set GPU mode via envycontrol (hybrid, integrated, nvidia)
    GpuMode(GpuModeParams),
}

#[derive(Parser)]
struct PowerParams {
    /// battery/plugged in
    ac_state: AcState,
    /// power mode (0=balanced, 1=gaming, 2=creator, 3=silent, 4=custom)
    #[arg(value_parser = clap::value_parser!(u8).range(0..=4))]
    pwr: u8,
    /// cpu boost (0=low, 1=medium, 2=high, 3=boost); required for custom
    #[arg(value_parser = clap::value_parser!(u8).range(0..=3))]
    cpu_mode: Option<u8>,
    /// gpu boost (0=low, 1=medium, 2=high); required for custom
    #[arg(value_parser = clap::value_parser!(u8).range(0..=2))]
    gpu_mode: Option<u8>,
}

#[derive(Parser)]
struct FanParams {
    /// battery/plugged in
    ac_state: AcState,
    /// fan speed in RPM (0 = automatic)
    speed: i32,
}

#[derive(Parser)]
struct BrightnessParams {
    /// battery/plugged in
    ac_state: AcState,
    /// brightness (0-100)
    #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
    brightness: u8,
}

#[derive(Parser)]
struct LogoParams {
    /// battery/plugged in
    ac_state: AcState,
    /// logo mode (0=off, 1=on, 2=breathing)
    #[arg(value_parser = clap::value_parser!(u8).range(0..=2))]
    logo_state: u8,
}

#[derive(Parser)]
struct IdleParams {
    /// battery/plugged in
    ac_state: AcState,
    /// minutes before the lights turn off (0 = never)
    minutes: u32,
}

#[derive(Parser)]
struct SyncParams {
    sync_state: OnOff,
}

#[derive(Parser)]
struct BhoParams {
    state: OnOff,
    /// charging threshold
    threshold: Option<u8>,
}

#[derive(Parser)]
struct RuntimePmParams {
    /// on (suspend dGPU) or off (keep active)
    state: OnOff,
}

#[derive(Parser)]
struct GpuModeParams {
    /// GPU mode: hybrid, integrated, or nvidia
    mode: String,
}

#[derive(ValueEnum, Clone)]
enum AcState {
    /// battery
    Bat,
    /// plugged in
    Ac,
}

impl AcState {
    fn as_index(&self) -> usize {
        match self {
            AcState::Bat => 0,
            AcState::Ac => 1,
        }
    }
}

#[derive(Parser, Clone)]
struct AcStateParam {
    /// battery/plugged in
    ac_state: AcState,
}

#[derive(Subcommand)]
enum StandardEffect {
    Off,
    Wave(WaveParams),
    Reactive(ReactiveParams),
    Breathing(BreathingParams),
    Spectrum,
    Static(StaticParams),
    Starlight(StarlightParams),
}

#[derive(Parser)]
struct WaveParams {
    /// direction (1 or 2)
    #[arg(value_parser = clap::value_parser!(u8).range(1..=2))]
    direction: u8,
}

#[derive(Parser)]
struct ReactiveParams {
    /// speed (1-4)
    #[arg(value_parser = clap::value_parser!(u8).range(1..=4))]
    speed: u8,
    /// red (0-255)
    red: u8,
    /// green (0-255)
    green: u8,
    /// blue (0-255)
    blue: u8,
}

/// Breathing and Starlight colors: kind 1 (single) takes the first color, 2 (dual) both, and
/// 3 (random) none.
#[derive(Parser)]
struct EffectColors {
    /// red1 (0-255)
    red1: Option<u8>,
    /// green1 (0-255)
    green1: Option<u8>,
    /// blue1 (0-255)
    blue1: Option<u8>,
    /// red2 (0-255)
    red2: Option<u8>,
    /// green2 (0-255)
    green2: Option<u8>,
    /// blue2 (0-255)
    blue2: Option<u8>,
}

#[derive(Parser)]
struct BreathingParams {
    /// kind (1=single, 2=dual, 3=random)
    #[arg(value_parser = clap::value_parser!(u8).range(1..=3))]
    kind: u8,
    #[command(flatten)]
    colors: EffectColors,
}

#[derive(Parser)]
struct StarlightParams {
    /// kind (1=single, 2=dual, 3=random)
    #[arg(value_parser = clap::value_parser!(u8).range(1..=3))]
    kind: u8,
    /// speed (1-3)
    #[arg(value_parser = clap::value_parser!(u8).range(1..=3))]
    speed: u8,
    #[command(flatten)]
    colors: EffectColors,
}

#[derive(Subcommand)]
enum Effect {
    Static(StaticParams),
    StaticGradient(StaticGradientParams),
    WaveGradient(WaveGradientParams),
    BreathingSingle(BreathingSingleParams),
}

#[derive(Parser)]
struct StaticParams {
    /// red (0-255)
    red: u8,
    /// green (0-255)
    green: u8,
    /// blue (0-255)
    blue: u8,
}

#[derive(Parser)]
struct StaticGradientParams {
    /// red1 (0-255)
    red1: u8,
    /// green1 (0-255)
    green1: u8,
    /// blue1 (0-255)
    blue1: u8,
    /// red2 (0-255)
    red2: u8,
    /// green2 (0-255)
    green2: u8,
    /// blue2 (0-255)
    blue2: u8,
}

#[derive(Parser)]
struct WaveGradientParams {
    /// red1 (0-255)
    red1: u8,
    /// green1 (0-255)
    green1: u8,
    /// blue1 (0-255)
    blue1: u8,
    /// red2 (0-255)
    red2: u8,
    /// green2 (0-255)
    green2: u8,
    /// blue2 (0-255)
    blue2: u8,
}

#[derive(Parser)]
struct BreathingSingleParams {
    /// red (0-255)
    red: u8,
    /// green (0-255)
    green: u8,
    /// blue (0-255)
    blue: u8,
    /// duration (0-255)
    duration: u8,
}

fn main() {
    if std::fs::metadata(comms::socket_path()).is_err() {
        fail("Error. Socket doesn't exist. Is the daemon running?");
    }

    let cli = Cli::parse();

    match cli.args {
        Args::Read { attr } => match attr {
            ReadAttr::Fan(AcStateParam { ac_state }) => read_fan_rpm(ac_state.as_index()),
            ReadAttr::Power(AcStateParam { ac_state }) => read_power_mode(ac_state.as_index()),
            ReadAttr::Brightness(AcStateParam { ac_state }) => read_brightness(ac_state.as_index()),
            ReadAttr::Logo(AcStateParam { ac_state }) => read_logo_mode(ac_state.as_index()),
            ReadAttr::Idle(AcStateParam { ac_state }) => read_idle(ac_state.as_index()),
            ReadAttr::Sync => read_sync(),
            ReadAttr::Bho => read_bho(),
            ReadAttr::FanRpm => read_actual_fan_rpm(),
            ReadAttr::Gpu => read_gpu_status(),
            ReadAttr::Device => read_device(),
            ReadAttr::Effect => read_effect(),
            ReadAttr::Frame => read_frame(),
        },
        Args::Write { attr } => match attr {
            WriteAttr::Fan(FanParams { ac_state, speed }) => {
                write_fan_speed(ac_state.as_index(), speed)
            }
            WriteAttr::Power(PowerParams {
                ac_state,
                pwr,
                cpu_mode,
                gpu_mode,
            }) => write_pwr_mode(ac_state.as_index(), pwr, cpu_mode, gpu_mode),
            WriteAttr::Brightness(BrightnessParams {
                ac_state,
                brightness,
            }) => write_brightness(ac_state.as_index(), brightness),
            WriteAttr::Sync(SyncParams { sync_state }) => write_sync(sync_state.is_on()),
            WriteAttr::Logo(LogoParams {
                ac_state,
                logo_state,
            }) => write_logo_mode(ac_state.as_index(), logo_state),
            WriteAttr::Idle(IdleParams { ac_state, minutes }) => {
                write_idle(ac_state.as_index(), minutes)
            }
            WriteAttr::Bho(BhoParams { state, threshold }) => {
                validate_and_write_bho(threshold, state)
            }
            WriteAttr::RuntimePm(RuntimePmParams { state }) => write_runtime_pm(state.is_on()),
            WriteAttr::GpuMode(GpuModeParams { mode }) => write_gpu_mode(&mode),
        },
        Args::Effect { effect } => {
            let (name, params) = match effect {
                Effect::Static(p) => ("static", vec![p.red, p.green, p.blue]),
                Effect::StaticGradient(p) => (
                    "static_gradient",
                    vec![p.red1, p.green1, p.blue1, p.red2, p.green2, p.blue2],
                ),
                Effect::WaveGradient(p) => (
                    "wave_gradient",
                    vec![p.red1, p.green1, p.blue1, p.red2, p.green2, p.blue2],
                ),
                Effect::BreathingSingle(p) => {
                    ("breathing_single", vec![p.red, p.green, p.blue, p.duration])
                }
            };
            apply(comms::DaemonCommand::SetEffect {
                name: name.to_string(),
                params,
            })
        }
        Args::StandardEffect { effect } => {
            let (name, params) = match effect {
                StandardEffect::Off => ("off", vec![]),
                StandardEffect::Spectrum => ("spectrum", vec![]),
                StandardEffect::Wave(p) => ("wave", vec![p.direction]),
                StandardEffect::Reactive(p) => ("reactive", vec![p.speed, p.red, p.green, p.blue]),
                StandardEffect::Static(p) => ("static", vec![p.red, p.green, p.blue]),
                StandardEffect::Breathing(p) => (
                    "breathing",
                    [vec![p.kind], p.colors.for_kind(p.kind)].concat(),
                ),
                StandardEffect::Starlight(p) => (
                    "starlight",
                    [vec![p.kind, p.speed], p.colors.for_kind(p.kind)].concat(),
                ),
            };
            apply(comms::DaemonCommand::SetStandardEffect {
                name: name.to_string(),
                params,
            })
        }
        Args::Wheel(WheelParams { direction, speed }) => {
            apply(comms::DaemonCommand::SetWheelEffect { direction, speed })
        }
        Args::AudioMeter(AudioMeterParams {
            color_mode,
            red,
            green,
            blue,
            sensitivity,
            decay,
            brightness,
        }) => apply(comms::DaemonCommand::SetSoundBarEffect {
            color_mode,
            r: red,
            g: green,
            b: blue,
            sensitivity,
            decay,
            brightness,
        }),
        Args::Stars(StarsParams { speed }) => apply(comms::DaemonCommand::SetStarsEffect { speed }),
        Args::Ripple(RippleParams {
            color_mode,
            red,
            green,
            blue,
            speed,
        }) => apply(comms::DaemonCommand::SetRippleEffect {
            color_mode,
            r: red,
            g: green,
            b: blue,
            speed,
        }),
        Args::Temperature(TemperatureParams { cool, hot }) => {
            if hot <= cool {
                Cli::command()
                    .error(ErrorKind::InvalidValue, "hot must be above cool")
                    .exit()
            }
            apply(comms::DaemonCommand::SetTemperatureEffect { cool, hot })
        }
        Args::Paint { action } => match action {
            Paint::Key(p) => {
                let cols = matrix_cols();
                if usize::from(p.row) >= service::MATRIX_ROWS || usize::from(p.col) >= cols {
                    Cli::command()
                        .error(
                            ErrorKind::InvalidValue,
                            format!(
                                "This keyboard's grid is {} rows by {cols} columns (both from 0)",
                                service::MATRIX_ROWS
                            ),
                        )
                        .exit()
                }
                apply(comms::DaemonCommand::SetCustomKey {
                    index: (usize::from(p.row) * cols + usize::from(p.col)) as u8,
                    r: p.red,
                    g: p.green,
                    b: p.blue,
                })
            }
            Paint::Fill(p) => apply(comms::DaemonCommand::FillCustomFrame {
                r: p.red,
                g: p.green,
                b: p.blue,
            }),
            Paint::Random => apply(comms::DaemonCommand::RandomizeCustomFrame),
            Paint::Clear => apply(comms::DaemonCommand::FillCustomFrame { r: 0, g: 0, b: 0 }),
        },
    }
}

impl EffectColors {
    /// The color bytes the given Breathing/Starlight kind sends: the firmware expects exactly
    /// these, not a fixed-width list.
    fn for_kind(&self, kind: u8) -> Vec<u8> {
        let first = [self.red1, self.green1, self.blue1];
        let second = [self.red2, self.green2, self.blue2];
        let wanted = match kind {
            1 => first.to_vec(),
            2 => [first, second].concat(),
            _ => vec![],
        };
        wanted
            .into_iter()
            .collect::<Option<Vec<u8>>>()
            .unwrap_or_else(|| {
                Cli::command()
                    .error(
                        ErrorKind::MissingRequiredArgument,
                        "kind 1 needs one RGB color and kind 2 needs two",
                    )
                    .exit()
            })
    }
}

/// Prints `msg` to stderr and exits with status 1.
fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

/// Sends a lighting command and reports whether the daemon applied it.
fn apply(command: comms::DaemonCommand) {
    use comms::DaemonResponse::*;
    match send_data(command) {
        Some(
            SetEffect { result }
            | SetStandardEffect { result }
            | SetWheelEffect { result }
            | SetSoundBarEffect { result }
            | SetStarsEffect { result }
            | SetRippleEffect { result }
            | SetTemperatureEffect { result }
            | SetCustomKey { result }
            | FillCustomFrame { result }
            | RandomizeCustomFrame { result },
        ) => {
            if result {
                println!("Effect set OK!");
            } else {
                fail("Effect set FAIL!");
            }
        }
        Some(_) => fail("Unexpected response from daemon!"),
        None => fail("Unknown daemon error!"),
    }
}

/// Columns in this laptop's lighting grid, from the device list entry for the detected model.
fn matrix_cols() -> usize {
    let name = match send_data(comms::DaemonCommand::GetDeviceName) {
        Some(comms::DaemonResponse::GetDeviceName { name }) => name,
        _ => fail("Unknown daemon error!"),
    };
    std::fs::read(service::device_file_path())
        .ok()
        .and_then(|json| serde_json::from_slice::<Vec<SupportedDevice>>(&json).ok())
        .and_then(|devices| devices.into_iter().find(|d| d.name == name))
        .map_or(16, |d| d.matrix_cols())
}

fn validate_and_write_bho(threshold: Option<u8>, state: OnOff) {
    match threshold {
        Some(threshold) => {
            if !valid_bho_threshold(threshold) {
                Cli::command()
                    .error(
                        ErrorKind::InvalidValue,
                        "Threshold must be multiple of 5 between 50 and 80",
                    )
                    .exit()
            }
            write_bho(state.is_on(), threshold)
        }
        None => {
            if state.is_on() {
                Cli::command()
                    .error(
                        ErrorKind::MissingRequiredArgument,
                        "Threshold is required when BHO is on",
                    )
                    .exit()
            }
            write_bho(state.is_on(), 80)
        }
    }
}

fn read_bho() {
    match send_data(comms::DaemonCommand::GetBatteryHealthOptimizer()) {
        Some(comms::DaemonResponse::GetBatteryHealthOptimizer { is_on, threshold }) => {
            if is_on {
                println!(
                    "Battery health optimization is on with a threshold of {}",
                    threshold
                );
            } else {
                println!("Battery health optimization is off");
            }
        }
        _ => fail("Unknown error occured when getting bho"),
    }
}

fn write_bho(is_on: bool, threshold: u8) {
    match send_data(comms::DaemonCommand::SetBatteryHealthOptimizer { is_on, threshold }) {
        Some(comms::DaemonResponse::SetBatteryHealthOptimizer { result: true }) => {
            if is_on {
                println!(
                    "Battery health optimization is on with a threshold of {}",
                    threshold
                );
            } else {
                println!("Successfully turned off bho");
            }
        }
        Some(comms::DaemonResponse::SetBatteryHealthOptimizer { result: false }) => {
            if is_on {
                fail(&format!(
                    "Failed to turn on bho with threshold of {}",
                    threshold
                ));
            } else {
                fail("Failed to turn off bho");
            }
        }
        _ => fail("Unknown error occured when toggling bho"),
    }
}

fn send_data(opt: comms::DaemonCommand) -> Option<comms::DaemonResponse> {
    match comms::bind() {
        Some(socket) => comms::send_to_daemon(opt, socket),
        None => {
            eprintln!("Error. Cannot bind to socket");
            None
        }
    }
}

fn read_fan_rpm(ac: usize) {
    match send_data(comms::DaemonCommand::GetFanSpeed { ac }) {
        Some(comms::DaemonResponse::GetFanSpeed { rpm }) => {
            let rpm_desc: String = match rpm {
                f if f < 0 => String::from("Unknown"),
                0 => String::from("Auto (0)"),
                _ => format!("{} RPM", rpm),
            };
            println!("Current fan setting: {}", rpm_desc);
        }
        Some(_) => fail("Daemon responded with invalid data!"),
        None => fail("Unknown daemon error!"),
    }
}

fn read_actual_fan_rpm() {
    match send_data(comms::DaemonCommand::GetActualFanRpm) {
        Some(comms::DaemonResponse::GetActualFanRpm { rpm }) => {
            println!("{}", rpm);
        }
        Some(_) => fail("Daemon responded with invalid data!"),
        None => fail("Unknown daemon error!"),
    }
}

fn read_logo_mode(ac: usize) {
    match send_data(comms::DaemonCommand::GetLogoLedState { ac }) {
        Some(comms::DaemonResponse::GetLogoLedState { logo_state }) => {
            let logo_state_desc: &str = match logo_state {
                0 => "Off",
                1 => "On",
                2 => "Breathing",
                _ => "Unknown",
            };
            println!("Current logo setting: {} ({})", logo_state_desc, logo_state);
        }
        Some(_) => fail("Daemon responded with invalid data!"),
        None => fail("Unknown daemon error!"),
    }
}

fn read_power_mode(ac: usize) {
    let pwr = match send_data(comms::DaemonCommand::GetPwrLevel { ac }) {
        Some(comms::DaemonResponse::GetPwrLevel { pwr }) => pwr,
        Some(_) => fail("Daemon responded with invalid data!"),
        None => fail("Unknown daemon error!"),
    };
    let power_desc: &str = match pwr {
        0 => "Balanced",
        1 => "Gaming",
        2 => "Creator",
        3 => "Silent",
        4 => "Custom",
        _ => "Unknown",
    };
    println!("Current power setting: {} ({})", power_desc, pwr);
    if pwr == 4 {
        if let Some(comms::DaemonResponse::GetCPUBoost { cpu }) =
            send_data(comms::DaemonCommand::GetCPUBoost { ac })
        {
            let cpu_boost_desc: &str = match cpu {
                0 => "Low",
                1 => "Medium",
                2 => "High",
                3 => "Boost",
                _ => "Unknown",
            };
            println!("Current CPU setting: {} ({})", cpu_boost_desc, cpu);
        }
        if let Some(comms::DaemonResponse::GetGPUBoost { gpu }) =
            send_data(comms::DaemonCommand::GetGPUBoost { ac })
        {
            let gpu_boost_desc: &str = match gpu {
                0 => "Low",
                1 => "Medium",
                2 => "High",
                _ => "Unknown",
            };
            println!("Current GPU setting: {} ({})", gpu_boost_desc, gpu);
        }
    }
}

fn write_pwr_mode(ac: usize, pwr_mode: u8, cpu_mode: Option<u8>, gpu_mode: Option<u8>) {
    let (cpu, gpu) = match (pwr_mode, cpu_mode, gpu_mode) {
        (4, Some(cpu), Some(gpu)) => (cpu, gpu),
        (4, ..) => Cli::command()
            .error(
                ErrorKind::MissingRequiredArgument,
                "Custom power mode (4) needs both a CPU and a GPU boost",
            )
            .exit(),
        (_, cpu, gpu) => (cpu.unwrap_or(0), gpu.unwrap_or(0)),
    };

    match send_data(comms::DaemonCommand::SetPowerMode {
        ac,
        pwr: pwr_mode,
        cpu,
        gpu,
    }) {
        Some(comms::DaemonResponse::SetPowerMode { result: true }) => read_power_mode(ac),
        Some(_) => fail("Daemon failed to apply the power mode"),
        None => fail("An error occurred while sending the command to the daemon"),
    }
}

fn read_brightness(ac: usize) {
    match send_data(comms::DaemonCommand::GetBrightness { ac }) {
        Some(comms::DaemonResponse::GetBrightness { result }) => {
            println!("Current brightness: {}", result);
        }
        Some(_) => fail("Daemon responded with invalid data!"),
        None => fail("Unknown daemon error!"),
    }
}

fn read_idle(ac: usize) {
    match send_data(comms::DaemonCommand::GetIdle { ac }) {
        Some(comms::DaemonResponse::GetIdle { minutes: 0 }) => {
            println!("Current idle timeout: never")
        }
        Some(comms::DaemonResponse::GetIdle { minutes }) => {
            println!("Current idle timeout: {} min", minutes)
        }
        Some(_) => fail("Daemon responded with invalid data!"),
        None => fail("Unknown daemon error!"),
    }
}

fn read_sync() {
    match send_data(comms::DaemonCommand::GetSync()) {
        Some(comms::DaemonResponse::GetSync { sync }) => {
            println!("Current sync: {:?}", sync);
        }
        Some(_) => fail("Daemon responded with invalid data!"),
        None => fail("Unknown daemon error!"),
    }
}

fn read_device() {
    let name = match send_data(comms::DaemonCommand::GetDeviceName) {
        Some(comms::DaemonResponse::GetDeviceName { name }) => name,
        Some(_) => fail("Daemon responded with invalid data!"),
        None => fail("Unknown daemon error!"),
    };
    println!("Model: {}", name);
    println!(
        "Lighting grid: {} x {}",
        service::MATRIX_ROWS,
        matrix_cols()
    );
    if let Some(comms::DaemonResponse::GetDeviceInfo {
        firmware,
        layout,
        serial,
    }) = send_data(comms::DaemonCommand::GetDeviceInfo)
    {
        if !firmware.is_empty() {
            println!("Keyboard firmware: {}", firmware);
        }
        println!(
            "Keyboard layout: {}",
            keyboard_layout_name(layout).unwrap_or("Unknown")
        );
        if !serial.is_empty() {
            println!("Serial number: {}", serial);
        }
    }
}

fn read_effect() {
    match send_data(comms::DaemonCommand::GetEffect) {
        Some(comms::DaemonResponse::GetEffect { name, params }) => {
            let params: Vec<String> = params.iter().map(u8::to_string).collect();
            println!("Current effect: {} {}", name, params.join(" "));
        }
        Some(_) => fail("Daemon responded with invalid data!"),
        None => fail("Unknown daemon error!"),
    }
}

fn read_frame() {
    match send_data(comms::DaemonCommand::GetKeyboardRGB { layer: -1 }) {
        Some(comms::DaemonResponse::GetKeyboardRGB { rgbdata, .. }) => {
            let cols = rgbdata.len() / 3 / service::MATRIX_ROWS;
            if cols == 0 {
                fail("This keyboard has no per-key lighting");
            }
            for row in rgbdata.chunks(cols * 3) {
                let keys: Vec<String> = row
                    .chunks(3)
                    .map(|c| format!("{:02x}{:02x}{:02x}", c[0], c[1], c[2]))
                    .collect();
                println!("{}", keys.join(" "));
            }
        }
        Some(_) => fail("Daemon responded with invalid data!"),
        None => fail("Unknown daemon error!"),
    }
}

fn write_brightness(ac: usize, val: u8) {
    match send_data(comms::DaemonCommand::SetBrightness { ac, val }) {
        Some(_) => read_brightness(ac),
        None => fail("Unknown error!"),
    }
}

fn write_fan_speed(ac: usize, x: i32) {
    match send_data(comms::DaemonCommand::SetFanSpeed { ac, rpm: x }) {
        Some(_) => read_fan_rpm(ac),
        None => fail("Unknown error!"),
    }
}

fn write_logo_mode(ac: usize, x: u8) {
    match send_data(comms::DaemonCommand::SetLogoLedState { ac, logo_state: x }) {
        Some(_) => read_logo_mode(ac),
        None => fail("Unknown error!"),
    }
}

fn write_idle(ac: usize, minutes: u32) {
    match send_data(comms::DaemonCommand::SetIdle { ac, val: minutes }) {
        Some(_) => read_idle(ac),
        None => fail("Unknown error!"),
    }
}

fn write_sync(sync: bool) {
    match send_data(comms::DaemonCommand::SetSync { sync }) {
        Some(_) => read_sync(),
        None => fail("Unknown error!"),
    }
}

fn read_gpu_status() {
    match send_data(comms::DaemonCommand::GetGpuStatus) {
        Some(comms::DaemonResponse::GetGpuStatus {
            gpus,
            dgpu_runtime_pm,
            envycontrol_mode,
            envycontrol_available,
        }) => {
            println!("Detected GPUs:");
            for gpu in &gpus {
                let type_label = if gpu.gpu_type == "dgpu" {
                    "dGPU"
                } else {
                    "iGPU"
                };
                println!(
                    "  {} [{}] {} (driver: {}, status: {})",
                    type_label, gpu.pci_slot, gpu.name, gpu.driver, gpu.runtime_status
                );
            }
            println!(
                "dGPU Runtime PM: {}",
                if dgpu_runtime_pm {
                    "auto (power saving)"
                } else {
                    "on (always active)"
                }
            );
            if envycontrol_available {
                println!("envycontrol mode: {}", envycontrol_mode);
            } else {
                println!("envycontrol: not installed");
            }
        }
        Some(_) => fail("Daemon responded with invalid data!"),
        None => fail("Unknown daemon error!"),
    }
}

fn write_runtime_pm(enabled: bool) {
    match send_data(comms::DaemonCommand::SetDgpuRuntimePM { enabled }) {
        Some(comms::DaemonResponse::SetDgpuRuntimePM { result: true }) => {
            println!(
                "dGPU runtime PM set to {}",
                if enabled {
                    "auto (power saving)"
                } else {
                    "on (always active)"
                }
            );
        }
        Some(comms::DaemonResponse::SetDgpuRuntimePM { result: false }) => {
            fail("Failed to set dGPU runtime PM (permission denied?)")
        }
        Some(_) => fail("Daemon responded with invalid data!"),
        None => fail("Unknown daemon error!"),
    }
}

fn write_gpu_mode(mode: &str) {
    match send_data(comms::DaemonCommand::SetGpuMode {
        mode: mode.to_string(),
    }) {
        Some(comms::DaemonResponse::SetGpuMode {
            result: true,
            message,
        }) => println!("{}", message),
        Some(comms::DaemonResponse::SetGpuMode {
            result: false,
            message,
        }) => fail(&format!("Failed: {}", message)),
        Some(_) => fail("Daemon responded with invalid data!"),
        None => fail("Unknown daemon error!"),
    }
}
