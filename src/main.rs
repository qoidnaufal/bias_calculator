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

pub(crate) fn par_extract(par: f64, known: f64) -> f64 {
    known * par / (known - par)
}

pub(crate) fn rcf(rcf1: f64, rcf2: f64) -> f64 {
    1.0 / (PI2 * rcf1 * rcf2)
}

// pub(crate) fn db(gain: f64) -> f64 {
//     20.0 * gain.log10()
// }

pub(crate) fn parse(s: &str) -> Result<f64, Error> {
    let parsed = if let Some(mega) = s.strip_suffix('M') {
        mega.parse::<f64>().map(|n| n * 1e6)
    }
    else if let Some(kilo) = s.strip_suffix(|c| c == 'k' || c == 'K') {
        kilo.parse::<f64>().map(|n| n * 1e3)
    }
    else if let Some(milli) = s.strip_suffix('m') {
        milli.parse::<f64>().map(|n| n * 1e-3)
    }
    else if let Some(micro) = s.strip_suffix('u') {
        micro.parse::<f64>().map(|n| n * 1e-6)
    }
    else if let Some(nano) = s.strip_suffix('n') {
        nano.parse::<f64>().map(|n| n * 1e-9)
    }
    else if let Some(pico) = s.strip_suffix('p') {
        pico.parse::<f64>().map(|n| n * 1e-12)
    }
    else {
        s.parse::<f64>()
    };

    parsed.map_err(|_| Error::NumParse(s.to_owned()))
}

#[derive(Debug)]
enum Error {
    NumParse(String),
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
            Error::NumParse(msg)           => write!(f, "Unable to parse int/float: {msg}"),
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
    -h, --help      : print help
    -ic             : set the desired collector current (default: 1mA)
    -beta, -hfe     : set the current gain (Hfe) (default: 100)
    -vcc            : set the supply voltage (default: 24)
    -ve             : set the desired emitter voltage (default: 2)
    -vc             : set the desired collector voltage (default: None)
    -vbe            : set the vbe drop of the device (default: 0.65)
    -vbias          : set the bias voltage if any (default: vcc)
    -hpass          : set the desired hi-pass filter frequency cutoff at the input (default: 7Hz)
    -lpass          : set the desired lo-pass filter for the collector-bypass or the collector-feedback capacitor (default: 20kHz)

Mode (default: ce)
    -cc             : set the mode of the device to common collector
    -ce             : set the mode of the device to common emitter

Bias (default: vdiv)
    -vdiv           : set the bias configuration to voltage divider bias
    -cfb            : set the bias configuration to collector feedback resistor
    -cfdiv          : set the bias configuration to collector feedback resistor with a base shunt-resistor
    -split          : set the bias configuration to split collector feedback with shunt network in between 2 feedback resistors
    -emb            : set the bias configuration to emitter bias
    -boost          : set to calculate emitter bypass capacitor
    -dz             : emulate 12ax7 cathode bypass using Dempwolf-Zolzer's model
    -koren          : emulate 12ax7 cathode bypass using Koren's model

Example:
    bjt -ic 500e-6 -vcc 23 -ve 2 -vbe 0.65 -beta 493.2 -ce -vdiv -vbias 9 -hpass 225
";

fn main() -> std::io::Result<()> {
    if let Err(err) = bjt::get_params().as_ref().map(bjt::analyze) {
        match err {
            Error::NeedsHelp => print!("{HELP}\n"),
            Error::NumParse(e) |
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
