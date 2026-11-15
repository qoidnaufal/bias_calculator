use std::io::Write;

mod bjt;
mod parser;
mod valve;
mod util;

const HELP: &'static str = r"
Usage:
    bias <Commands> <value> <Mode> <Config>

Device:
    -bjt            : Bipolar Junction Transistor
    -valve          : 12ax7

Commands
    -h, --help      : print help
    -ic             : set the desired collector current (default: 500uA)
    -beta, -hfe     : set the current gain (Hfe) (default: 100)
    -vcc            : set the supply voltage (default: 24)
    -ve             : set the desired emitter voltage (default: 2)
    -vc             : set the desired collector voltage (default: None)
    -vbe            : set the vbe drop of the device (default: 0.65)
    -vbias          : set the bias voltage if any (default: vcc)
    -hpass          : set the desired hi-pass filter frequency cutoff at the input (default: 3.3Hz)
    -lpass          : set the desired lo-pass filter for the Miller capacitor (default: 7.7kHz)

Mode (default: ce)
    -cc             : set the mode of the device to common collector
    -ce             : set the mode of the device to common emitter

Config (for bjt, default: vdiv)
    -vdiv           : set the bias configuration to voltage divider bias
    -cfb            : set the bias configuration to collector feedback resistor
    -cfdiv          : set the bias configuration to collector feedback resistor with a base shunt-resistor
    -split          : set the bias configuration to split collector feedback with shunt network in between 2 feedback resistors
    -fixed          : set the bias configuration to fixed bias
    -bts            : set the bias configuration to bootstrap
    -boost          : set to calculate emitter bypass capacitor
    -dz             : emulate 12ax7 cathode bypass using Dempwolf-Zolzer's model
    -koren          : emulate 12ax7 cathode bypass using Koren's model

Config (for 12ax7, default: koren)
    -dz             : calculate 12ax7 bias using Dempwolf-Zolzer's model
    -koren          : calculate 12ax7 bias using Koren's model

Example:
    bias -bjt -ic 500e-6 -vcc 23 -ve 2 -vbe 0.65 -beta 493.2 -ce -vdiv -vbias 9 -hpass 225
    bias -valve -vcc 392 -rp 150k -rk 1.5k -ck 470n
";

fn main() -> std::io::Result<()> {
    if let Err(err) = parser::get_params().as_ref().map(parser::Params::analyze) {
        match err {
            util::Error::NeedsHelp => print!("{HELP}\n"),
            util::Error::InvalidArgs(e) => {
                print!("[INVALID ARGS] {e}\n");
                print!("{HELP}\n");
            },
            util::Error::NumParse(s) => print!("[NUM PARSE ERROR] {s}\n"),
            other => print!("{other}\n"),
        }
        std::io::stdout().flush()?;
    }

    Ok(())
}
