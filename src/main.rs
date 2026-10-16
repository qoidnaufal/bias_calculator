use std::io::Write;

mod valve;
mod bjt;

const PI2: f64 = 2.0 * core::f64::consts::PI;

#[macro_export]
macro_rules! rpar {
    ($($r:expr),* $(,)?) => {{
        let mut g = 0.0;
        $(g += 1.0/$r;)*
        1.0/g
    }};
}

pub(crate) fn rcf(rcf1: f64, rcf2: f64) -> f64 {
    1.0 / (PI2 * rcf1 * rcf2)
}

// pub(crate) fn db(gain: f64) -> f64 {
//     20.0 * gain.log10()
// }

pub(crate) fn parse(s: &str) -> Result<f64, Error> {
    s.parse::<f64>().map_err(|_| Error::UnableToParseFloat(s.to_owned()))
}

#[derive(Debug)]
enum Error {
    UnableToParseFloat(String),
    InvalidArgs(String),
    Other(String),
    NeedsHelp,
    DiscouragedMode,
    Aborted,
    Io(std::io::Error),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::UnableToParseFloat(msg) => write!(f, "Unable to parse float: {msg}"),
            Error::InvalidArgs(msg)        => write!(f, "Invalid args: {msg}"),
            Error::Other(msg)              => write!(f, "Other: {msg}"),
            Error::NeedsHelp               => write!(f, "Needs help"),
            Error::DiscouragedMode         => write!(f, "This mode & bias is discouraged"),
            Error::Aborted                 => write!(f, "Aborted"),
            Error::Io(io)                  => write!(f, "Io: {io:?}"),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl std::error::Error for Error {}

pub(crate) struct SelectError;

const HELP: &'static str = r"
Usage:
    bjt <Commands> <value> <Mode> <Bias>

Commands
    -h, --help  : print help
    -ic         : set the desired collector current (default: 1mA)
    -beta, -hfe : set the current gain (Hfe) (default: 100)
    -load       : set the load impedance of the next stage (default: None)
    -vcc        : set the supply voltage (default: 24)
    -ve         : set the desired emitter voltage (default: 2)
    -vbe        : set the vbe drop of the device (default: 0.65)
    -vbias      : set the bias voltage if any (default: vcc)
    -hpass      : set the desired hi-pass filter frequency cutoff at the input (default: 7Hz)
    -lpass      : set the desired lo-pass filter for the collector-bypass or the collector-feedback capacitor (default: 20kHz)
    -boost      : this is used if you want to emulate the boost provided by cathode bypass capacitor on a 12ax7
    -cgk        : this is used if you want to emulate the frequency roll-off by a capacitor connected between grid and cathode on a 12ax7
    -cpg        : this is used if you want to emulate the frequency roll-off by a capacitor connected between plate and vsupply / ground on a 12ax7

Mode (default: ce)
    -cc         : set the mode of the device to common collector
    -ce         : set the mode of the device to common emitter

Bias (default: voltdiv)
    -vdiv       : set the bias configuration to voltage divider bias
    -cfb        : set the bias configuration to collector feedback resistor
    -cfdiv      : set the bias configuration to collector feedback resistor with a base shunt-resistor
    -split      : set the bias configuration to split collector feedback with shunt network in between 2 feedback resistors
    -emb        : set the bias configuration to emitter bias

Example:
    bjt -ic 500e-6 -vcc 23 -ve 2 -vbe 0.65 -beta 493.2 -ce -vdiv -vbias 9 -hpass 225
";

fn main() -> std::io::Result<()> {
    if let Err(err) = bjt::get_params().as_ref().map(bjt::analyze) {
        match err {
            Error::NeedsHelp => print!("{HELP}\n"),
            Error::UnableToParseFloat(e) |
            Error::InvalidArgs(e) => {
                print!("[ERROR] {e}\n");
                print!("{HELP}\n");
            },
            other => print!("{other}\n"),
        }
        std::io::stdout().flush()?;
    }

    Ok(())
}
