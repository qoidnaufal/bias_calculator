const PI2:         f64 = 2.0 * core::f64::consts::PI;
const BOLTZMANN:   f64 = 1.380649e-23;
const CHARGE:      f64 = 1.602176634e-19;
const TEMPERATURE: f64 = 300.15;
const VT:          f64 = BOLTZMANN * TEMPERATURE / CHARGE;

#[derive(Debug)]
enum Error {
    UnableToParseFloat(String),
    InvalidArgs(String),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::UnableToParseFloat(msg) => write!(f, "Unable to parse float: {msg}"),
            Error::InvalidArgs(msg)        => write!(f, "Invalid args: {msg}"),
        }
    }
}

struct Params {
    load1:    Option<f64>,
    load2:    Option<f64>,
    ic:       f64,
    beta:     f64,
    ibias_x:  f64,
    i_cutoff: f64,
    o_cutoff: f64,
    rc_lpass: f64,
    vcc:      f64,
    vbe:      f64,
    ve:       f64,
    vbias:    Option<f64>,
    mode:     BjtMode,
    bias:     BiasConfig,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            load1:    None,
            load2:    None,
            ic:       1e-3,
            beta:     1e2,
            ibias_x:  10.0,
            i_cutoff: 1.0,
            o_cutoff: 1.0,
            rc_lpass: 6.6e3,
            vcc:      24.0,
            vbe:      0.65,
            ve:       1.0,
            vbias:    None,
            mode:     BjtMode::CommonEmitter,
            bias:     BiasConfig::VoltageDivider,
        }
    }
}

enum BjtMode {
    CommonEmitter,
    CommonCollector,
}

enum BiasConfig {
    VoltageDivider,
    CollectorFeedback,
    FixedBias,
}

fn get_mode_and_bias(mode: &mut BjtMode, bias: &mut BiasConfig, s: &str) -> Result<(), Error> {
    match s {
        "-ce"        => *mode = BjtMode::CommonEmitter,
        "-cc"        => *mode = BjtMode::CommonCollector,
        "-voltdiv"   => *bias = BiasConfig::VoltageDivider,
        "-feedback"  => *bias = BiasConfig::CollectorFeedback,
        "-fixed"     => *bias = BiasConfig::FixedBias,
        _ => return Err(Error::InvalidArgs(s.to_owned()))
    }

    Ok(())
}

impl core::fmt::Display for BjtMode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            BjtMode::CommonEmitter => "Common Emitter",
            BjtMode::CommonCollector => "Common Collector",
        };
        write!(f, "{s}")
    }
}

impl core::fmt::Display for BiasConfig {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            BiasConfig::VoltageDivider => "Voltage Divider",
            BiasConfig::CollectorFeedback => "Collector Feedback",
            BiasConfig::FixedBias => "Fixed Bias",
        };
        write!(f, "{s}")
    }
}

fn rpar(r1: f64, r2: f64) -> f64 {
    r1 * r2 / (r1 + r2)
}

fn parse(s: &str) -> Result<f64, Error> {
    s.parse::<f64>().map_err(|_| Error::UnableToParseFloat(s.to_owned()))
}

fn get_params() -> Result<Params, Error> {
    let mut params = Params::default();
    let args = std::env::args().collect::<Vec<_>>();

    if let Some(cmd) = args.get(1..) {
        let len = cmd.len();
        let mut i = 0;

        while i < len {
            match cmd[i].as_str() {
                "-ic"     => { params.ic = parse(&cmd[i + 1])?; i += 1 }
                "-beta"   => { params.beta = parse(&cmd[i + 1])?; i += 1 },
                "-ibias"  => { params.ibias_x = parse(&cmd[i + 1])?; i += 1 },
                "-load1"  => { params.load1 = Some(parse(&cmd[i + 1])?); i += 1 },
                "-load2"  => { params.load2 = Some(parse(&cmd[i + 1])?); i += 1 },
                "-icut"   => { params.i_cutoff = parse(&cmd[i + 1])?; i += 1 },
                "-ocut"   => { params.o_cutoff = parse(&cmd[i + 1])?; i += 1 },
                "-lpass"  => { params.rc_lpass = parse(&cmd[i + 1])?; i += 1 },
                "-vcc"    => { params.vcc = parse(&cmd[i + 1])?; i += 1 },
                "-ve"     => { params.ve = parse(&cmd[i + 1])?; i += 1 },
                "-vbe"    => { params.vbe = parse(&cmd[i + 1])?; i += 1 },
                "-vbias"  => { params.vbias = Some(parse(&cmd[i + 1])?); i += 1 },
                other     => get_mode_and_bias(&mut params.mode, &mut params.bias, other)?,
            }
            i += 1;
        }
    }

    Ok(params)
}

fn rcf(rcf1: f64, rcf2: f64) -> f64 {
    1.0 / (PI2 * rcf1 * rcf2)
}

fn analyze(params: &Params) {
    println!("\x1B[1;32m{}\x1B[0m - \x1B[1;32m{}\x1B[0m", params.mode, params.bias);
    println!(" ---------------");

    let ib = params.ic / params.beta;
    let ie = params.ic + ib;
    let re = params.ve / ie;
    let vb = params.ve + params.vbe;

    let (vc, rc) = match params.mode {
        BjtMode::CommonEmitter => {
            let vrc = (params.vcc - params.ve) / 2.0;
            let vc = params.vcc - vrc;
            let rc = vrc / params.ic;
            println!("expected vc      : {vc:.2}");
            println!("expected vb      : {vb:.2}");
            println!(" ---------------");
            (vc, rc)
        },
        BjtMode::CommonCollector => (params.vcc, 0.0),
    };

    let vbias = params.vbias.unwrap_or(params.vcc);
    let ibias = params.ibias_x * ib;

    let rpi = VT * params.beta / params.ic;
    let base_impedance = rpi + (params.beta + 1.0) * re;

    println!("rc               : {rc:.2}");
    println!("re               : {re:.2}");

    let r_bias = match params.bias {
        BiasConfig::VoltageDivider => {
            let r1 = (vbias - vb) / (ibias + ib);
            let r2 = vb / ibias;

            println!("r1               : {r1:.1}");
            println!("r2               : {r2:.2}");
            println!("bias current     : {ibias:.2e}");

            rpar(r1, r2)
        },
        BiasConfig::CollectorFeedback => {
            (vc - vb) / ib
        },
        BiasConfig::FixedBias => {
            (vbias - vb) / (ibias + ib)
        },
    };

    // let rout = rdiv / params.beta;

    println!("rpi              : {rpi:.2e}");
    println!("base impedance   : {base_impedance:.2e}");

    println!("input impedance  : {:.2e}", rpar(base_impedance, r_bias));
    // println!("output impedance : {rout:.2e}");
    println!("r_bias           : {r_bias:.2e}");
    println!("input cap        : {:.2e}", rcf(r_bias, params.i_cutoff));

    match params.mode {
        BjtMode::CommonCollector => {},
        BjtMode::CommonEmitter => {
            println!("low pass cap     : {:.2e}", rcf(rc, params.rc_lpass));
        },
    }

    if let Some(load1) = params.load1 && let Some(load2) = params.load2 {
        let par = rpar(load1, load2);
        println!("output cap       : {:.2e}", rcf(par, params.o_cutoff));
    } else if let Some(load1) = params.load1 {
        println!("output cap       : {:.2e}", rcf(load1, params.o_cutoff));
    };
}

const HELP: &'static str = r"
Usage:
    bjt <Commands> <value> <Mode> <Bias>

Commands:
    -ic       : set the desired collector current (default: 1mA)
    -beta     : set the current gain (Hfe) (default: 100)
    -ibias    : set the desired bias current multiplier with regards to base current (default: 10x)
    -load1    : set the load impedance of the next stage (parallel with output impedance)
    -load2    : set the load impedance of the next stage (parallel with load1)
    -icut     : set the desired high pass filter frequency cutoff at the input (default: 1)
    -ocut     : set the desired high pass filter frequency cutoff at the output (default: 1)
    -vcc      : set the supply voltage (default: 24)
    -ve       : set the desired emitter voltage (default: 1)
    -vbe      : set the vbe drop of the device (default: 0.65)
    -vbias    : set the bias voltage if any (default: vcc)

Mode (default: ce):
    -cc       : set the mode of the device to common collector
    -ce       : set the mode of the device to common emitter

Bias (default: voltdiv):
    -voltdiv  : set the bias configuration to voltage divider bias
    -feedback : set the bias configuration to collectod feedback resistor
    -fixed    : set the bias configuration to fixed bias
";

fn main() {
    match get_params() {
        Ok(params) => analyze(&params),
        Err(e) => {
            println!("ERROR: {e}");
            println!("{HELP}");
        },
    }
}
