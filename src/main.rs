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
            i_cutoff: 7.0,
            o_cutoff: 7.0,
            rc_lpass: 6.6e3,
            vcc:      24.0,
            vbe:      0.65,
            ve:       2.4,
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
    EmitterBias,
}

fn get_mode_and_bias(mode: &mut BjtMode, bias: &mut BiasConfig, s: &str) -> Result<(), Error> {
    match s {
        "-ce"        => *mode = BjtMode::CommonEmitter,
        "-cc"        => *mode = BjtMode::CommonCollector,
        "-vdv"       => *bias = BiasConfig::VoltageDivider,
        "-feedback"  => *bias = BiasConfig::CollectorFeedback,
        "-emb"       => *bias = BiasConfig::EmitterBias,
        _ => return Err(Error::InvalidArgs(s.to_owned()))
    }

    Ok(())
}

impl core::fmt::Display for BjtMode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            BjtMode::CommonEmitter   => "Common Emitter",
            BjtMode::CommonCollector => "Common Collector",
        };
        write!(f, "{s}")
    }
}

impl core::fmt::Display for BiasConfig {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            BiasConfig::VoltageDivider    => "Voltage Divider",
            BiasConfig::CollectorFeedback => "Collector Feedback",
            BiasConfig::EmitterBias       => "Emitter Bias",
        };
        write!(f, "{s}")
    }
}

macro_rules! rpar {
    ($($r:expr),* $(,)?) => {{
        let mut gp: f64 = 0.0;
        $(
            gp += 1.0 / $r;
        )*
        1.0 / gp
    }};
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

#[derive(Clone, Copy)]
struct VoltageDivider {
    r1: f64,
    r2: f64,
    ibias: f64,
}

union Rbias {
    vdv: VoltageDivider,
    rb: f64
}

impl Rbias {
    fn get(&self, bias: &BiasConfig) -> f64 {
        match bias {
            BiasConfig::VoltageDivider => unsafe {
                let r1 = self.vdv.r1;
                let r2 = self.vdv.r2;
                println!("r1               : {r1:.2e}");
                println!("r2               : {r2:.2e}");
                println!("ibias            : {:.2e}", self.vdv.ibias);
                rpar!(r1, r2)
            },
            BiasConfig::CollectorFeedback
            | BiasConfig::EmitterBias => unsafe {
                let rb = self.rb;
                println!("rb               : {rb:.2}");
                rb
            }
        }
    }
}

fn select_r2(r2: &mut f64) {
    use core::str::FromStr;
    use std::io::{Read, Write};

    let mut stdin = std::io::stdin();

    loop {
        print!("Max r2 is: \x1B[1;33m{r2:.2e}\x1B[0m. Pick your desired r2 value: ");
        std::io::stdout().flush().unwrap();
        let mut buffer = [0u8; 32];
        match stdin.read(&mut buffer) {
            Ok(n) => {
                if let Ok(Ok(new)) = str::from_utf8(&buffer[..n])
                    .map(str::trim)
                    .map(<f64 as FromStr>::from_str)
                {
                        let old = *r2;
                        if new > old {
                            print!("New r2 exceeds maximum recommended value, \x1B[1;33m{old:.2e}\x1B[0m is used.\n");
                            std::io::stdout().flush().unwrap();
                        }
                        let new_r2 = new.min(old);
                        *r2 = new_r2;
                        break;
                }
            },
            Err(_) => {}
        }
    }
}

fn select_re(re: &mut f64) {
    use core::str::FromStr;
    use std::io::{Read, Write};

    let mut stdin = std::io::stdin();

    loop {
        print!("Suggested re is: \x1B[1;33m{re:.2e}\x1B[0m. Pick your desired re value: ");
        std::io::stdout().flush().unwrap();
        let mut buffer = [0u8; 32];
        match stdin.read(&mut buffer) {
            Ok(n) => {
                if let Ok(Ok(new)) = str::from_utf8(&buffer[..n])
                    .map(str::trim)
                    .map(<f64 as FromStr>::from_str)
                {
                        *re = new;
                        break;
                }
            },
            Err(_) => {}
        }
    }
}

struct Currents {
    ic: f64,
    ib: f64,
    ie: f64,
}

fn finetune_initial_condition(params: &Params) -> (Currents, f64) {
    let mut ib = params.ic / params.beta;
    let mut ie = params.ic + ib;
    let mut re = params.ve / ie;
    select_re(&mut re);
    ie = params.ve / re;
    ib = ie / (params.beta + 1.0);
    let ic = ib * params.beta;

    (Currents { ic, ib, ie }, re)
}

fn analyze(params: &Params) {
    let (currents, re) = finetune_initial_condition(&params);
    let vb = params.ve + params.vbe;

    let (vc, rc) = match params.mode {
        BjtMode::CommonEmitter => {
            let vrc = (params.vcc - params.ve) / 2.0;
            let vc = params.vcc - vrc;
            let rc = vrc / params.ic;
            (vc, rc)
        },
        BjtMode::CommonCollector => (params.vcc, 0.0),
    };

    let vbias = params.vbias.unwrap_or(params.vcc);

    let r_bias = match params.bias {
        BiasConfig::VoltageDivider => {
            let mut r2 = 0.1 * params.beta * re;
            select_r2(&mut r2);
            let ir2 = vb / r2;
            let ibias = currents.ib + ir2;
            let r1 = (vbias - vb) / ibias;

            Rbias {
                vdv: VoltageDivider { r1, r2, ibias }
            }
        },
        BiasConfig::CollectorFeedback => Rbias { rb: (vc - params.vbe) / currents.ib },
        BiasConfig::EmitterBias => Rbias { rb: (vbias - vb) / currents.ib },
    };

    let vce = vc - params.ve;

    println!("\x1B[1;32m{}\x1B[0m - \x1B[1;32m{}\x1B[0m", params.mode, params.bias);
    println!("vc               : {vc:.2}");
    if matches!(params.mode, BjtMode::CommonEmitter) {
        println!("vce              : {vce:.2}");
    }
    println!("ic               : {:.2e}", currents.ic);
    println!("-----------------------------------");

    if matches!(params.mode, BjtMode::CommonEmitter) {
        println!("rc               : {rc:.2}");
    }
    println!("re               : {re:.2}");

    let r_bias = r_bias.get(&params.bias);
    let ree = VT / currents.ie;
    // let ro = vce / params.ic;

    let zi = match params.bias {
        BiasConfig::VoltageDivider => {
            // let rpi = VT * params.beta / params.ic;
            // let zb = rpi + (params.beta + 1.0) * re;
            let zb = params.beta * ree;
            rpar!(r_bias, zb)
        },
        BiasConfig::CollectorFeedback => {
            let beta_inv = 1.0 / params.beta;
            let denom = beta_inv + (re + rc) / r_bias;
            ree / denom
        },
        BiasConfig::EmitterBias => {
            let zb = params.beta * ree + (params.beta + 1.0) * re;
            rpar!(r_bias, zb)
        },
    };

    let zo = match params.bias {
        BiasConfig::VoltageDivider => rc,
        BiasConfig::CollectorFeedback => rpar!(r_bias, rc),
        BiasConfig::EmitterBias => rpar!(re, ree),
    };

    println!("zi               : {zi:.2e}");
    println!("zo               : {zo:.2e}");
    println!("input cap        : {:.2e}", rcf(zi, params.i_cutoff));

    if matches!(params.mode, BjtMode::CommonEmitter) {
        println!("low pass cap     : {:.2e}", rcf(rc, params.rc_lpass));
    }

    if let Some(load1) = params.load1 {
        if let Some(load2) = params.load2 {
            let par = rpar!(load1, load2);
            println!("output cap       : {:.2e}", rcf(par, params.o_cutoff));
        } else {
            println!("output cap       : {:.2e}", rcf(load1, params.o_cutoff));
        }
    } else {
    };
}

const HELP: &'static str = r"
Usage:
    bjt <Commands> <value> <Mode> <Bias>

Commands:
    -ic        : set the desired collector current (default: 1mA)
    -beta      : set the current gain (Hfe) (default: 100)
    -load1     : set the load impedance of the next stage (parallel with output impedance)
    -load2     : set the load impedance of the next stage (parallel with load1)
    -icut      : set the desired high pass filter frequency cutoff at the input (default: 1)
    -ocut      : set the desired high pass filter frequency cutoff at the output (default: 1)
    -vcc       : set the supply voltage (default: 24)
    -ve        : set the desired emitter voltage (default: 1)
    -vbe       : set the vbe drop of the device (default: 0.65)
    -vbias     : set the bias voltage if any (default: vcc)

Mode (default: ce):
    -cc        : set the mode of the device to common collector
    -ce        : set the mode of the device to common emitter

Bias (default: voltdiv):
    -vdv       : set the bias configuration to voltage divider bias
    -feedback  : set the bias configuration to collectod feedback resistor
    -emb       : set the bias configuration to emitter bias
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
