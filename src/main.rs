use std::io::Write;

const PI2:         f64 = 2.0 * core::f64::consts::PI;
const BOLTZMANN:   f64 = 1.380649e-23;
const CHARGE:      f64 = 1.602176634e-19;
const TEMPERATURE: f64 = 300.15;
const VT:          f64 = BOLTZMANN * TEMPERATURE / CHARGE;

macro_rules! rpar {
    ($($r:expr),* $(,)?) => {{
        let mut gp: f64 = 0.0;
        $(
            gp += 1.0 / $r;
        )*
        1.0 / gp
    }};
}

#[derive(Debug)]
enum Error {
    UnableToParseFloat(String),
    InvalidArgs(String),
    NeedsHelp,
    DiscouragedMode,
    Aborted,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::UnableToParseFloat(msg) => write!(f, "Unable to parse float: {msg}"),
            Error::InvalidArgs(msg)        => write!(f, "Invalid args: {msg}"),
            Error::NeedsHelp               => write!(f, "Needs help"),
            Error::DiscouragedMode         => write!(f, "This mode & bias is discouraged"),
            Error::Aborted                 => write!(f, "Aborted"),
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

impl Params {
    fn vb(&self) -> f64 {
        self.ve + self.vbe
    }

    /// vc = (vcc + ve) / 2
    /// vrc = vcc - vc = vcc - [(vcc + ve) / 2]
    /// vrc = (vcc - ve)/2
    fn vrc(&self) -> f64 {
        (self.vcc - self.ve) / 2.0
    }

    fn v_bias(&self) -> f64 {
        self.vbias.unwrap_or(self.vcc)
    }

    // fn rpi(&self) -> f64 {
    //     VT * self.beta / self.ic
    // }
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

/// Vcc - Ibc * Rc - Ib * Rf - Vbe - Ie * Re = 0
fn collector_feedback(params: &Params) -> Result<(), Error> {
    let mut ib = params.ic / params.beta;
    let mut ie = params.ic + ib;
    let mut re = params.ve / ie;
    select("Suggested re", re, |new| re = new)?;
    ie = params.ve / re;
    ib = ie / (params.beta + 1.0);
    let ic = ib * params.beta;
    let ibc = ib + ic;

    let vrc = params.vrc();
    let vb = params.vb();
    let vc = params.vcc - vrc;
    let rc = vrc / ibc;

    let rf = (vc - vb) / ib;
    let ree = VT / ie;

    let beta_inv = 1.0 / params.beta;
    let denom = beta_inv + (re + rc) / rf;
    let zi = ree / denom;
    let zo = rpar!(rf, rc);

    let input_cap = rcf(zi, params.i_cutoff);
    let low_pass_cap = rcf(zi, params.rc_lpass);

    print_mode(&params.mode, &params.bias);
    println!("vc               : {vc:.2}");
    println!("ic               : {ic:.2e}");
    println!("ibc              : {ibc:.2e}");
    println!("vce              : {:.2}", vc - params.ve);
    println!("-----------------------------------");
    println!("rc               : {rc:.2e}");
    println!("re               : {re:.2e}");
    println!("rf               : {rf:.2e}");
    println!("zi               : {zi:.2e}");
    println!("zo               : {zo:.2e}");
    println!("input cap        : {input_cap:.2e}");
    println!("low pass cap     : {low_pass_cap:.2e}");

    Ok(())
}

fn voltage_divider_ce(params: &Params) -> Result<(), Error> {
    let mut ib = params.ic / params.beta;
    let mut ie = params.ic + ib;
    let mut re = params.ve / ie;
    select("Suggested re", re, |new| re = new)?;
    ie = params.ve / re;
    ib = ie / (params.beta + 1.0);
    let ic = ib * params.beta;

    let vb = params.vb();
    let vrc = params.vrc();
    let vc = params.vcc - vrc;
    let rc = vrc / ic;

    let mut r2 = 0.1 * params.beta * re;
    select("Max r2", r2, |new| {
        if new > r2 {
            print!("New r2 exceeds maximum recommended value, \x1B[1;33m{r2:.2e}\x1B[0m is used.\n");
            std::io::stdout().flush().unwrap();
        }
        r2 = new.min(r2);
    })?;
    let ir2 = vb / r2;
    let ir1 = ib + ir2;
    let r1 = (params.v_bias() - vb) / ir1;
    let r_bias = rpar!(r1, r2);
    let ree = VT / ie;

    let zi = rpar!(params.beta * ree, r_bias);
    let input_cap = rcf(zi, params.i_cutoff);
    let low_pass_cap = rcf(rc, params.rc_lpass);

    print_mode(&params.mode, &params.bias);
    println!("vc               : {vc:.2}");
    println!("ic               : {ic:.2e}");
    println!("ir1              : {ir1:.2e}");
    println!("vce              : {:.2}", vc - params.ve);
    println!("-----------------------------------");
    println!("rc               : {rc:.2e}");
    println!("re               : {re:.2e}");
    println!("r1               : {r1:.2e}");
    println!("r2               : {r2:.2e}");
    println!("zi               : {zi:.2e}");
    println!("zo               : {rc:.2e}");
    println!("input cap        : {input_cap:.2e}");
    println!("low pass cap     : {low_pass_cap:.2e}");

    Ok(())
}

fn voltage_divider_cc(params: &Params) -> Result<(), Error> {
    let mut ib = params.ic / params.beta;
    let mut ie = params.ic + ib;
    let mut re = params.ve / ie;
    select("Suggested re", re, |new| re = new)?;
    ie = params.ve / re;
    ib = ie / (params.beta + 1.0);
    let ic = ib * params.beta;

    let vb = params.vb();

    let mut r2 = 0.1 * params.beta * re;
    select("Max r2", r2, |new| {
        if new > r2 {
            print!("New r2 exceeds maximum recommended value, \x1B[1;33m{r2:.2e}\x1B[0m is used.\n");
            std::io::stdout().flush().unwrap();
        }
        r2 = new.min(r2);
    })?;
    let ir2 = vb / r2;
    let ir1 = ib + ir2;
    let r1 = (params.v_bias() - vb) / ir1;
    let r_bias = rpar!(r1, r2);
    let ree = VT / ie;

    let zi = rpar!(params.beta * ree, r_bias);
    let zo = rpar!(re, ree);
    let input_cap = rcf(zi, params.i_cutoff);

    print_mode(&params.mode, &params.bias);
    println!("ic               : {ic:.2e}");
    println!("ir1              : {ir1:.2e}");
    println!("-----------------------------------");
    println!("re               : {re:.2e}");
    println!("r1               : {r1:.2e}");
    println!("r2               : {r2:.2e}");
    println!("zi               : {zi:.2e}");
    println!("zo               : {zo:.2e}");
    println!("input cap        : {input_cap:.2e}");

    Ok(())
}

fn emitter_bias_cc(params: &Params) -> Result<(), Error> {
    let mut ib = params.ic / params.beta;
    let mut ie = params.ic + ib;
    let mut re = params.ve / ie;
    select("Suggested re", re, |new| re = new)?;
    ie = params.ve / re;
    ib = ie / (params.beta + 1.0);

    let vb = params.vb();

    let mut rb = (params.v_bias() - vb) / ib;
    select("Suggested rb", rb, |new| rb = new)?;
    let ree = VT / ie;

    let zb = params.beta * ree + (params.beta + 1.0) * re;
    let zi = rpar!(zb, rb);
    let zo = rpar!(re, ree);
    let input_cap = rcf(zi, params.i_cutoff);

    print_mode(&params.mode, &params.bias);
    println!("ic               : {:.2e}", params.ic);
    println!("ib               : {ib:.2e}");
    println!("-----------------------------------");
    println!("re               : {re:.2e}");
    println!("rb               : {rb:.2e}");
    println!("zi               : {zi:.2e}");
    println!("zo               : {zo:.2e}");
    println!("input cap        : {input_cap:.2e}");

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

fn parse(s: &str) -> Result<f64, Error> {
    s.parse::<f64>().map_err(|_| Error::UnableToParseFloat(s.to_owned()))
}

fn get_mode_and_bias(mode: &mut BjtMode, bias: &mut BiasConfig, s: &str) -> Result<(), Error> {
    match s {
        "-ce"            => *mode = BjtMode::CommonEmitter,
        "-cc"            => *mode = BjtMode::CommonCollector,
        "-vdv"           => *bias = BiasConfig::VoltageDivider,
        "-feedback"      => *bias = BiasConfig::CollectorFeedback,
        "-emb"           => *bias = BiasConfig::EmitterBias,
        "-h" | "--help"  => return Err(Error::NeedsHelp),
        _                => return Err(Error::InvalidArgs(s.to_owned()))
    }

    Ok(())
}

fn get_params() -> Result<Params, Error> {
    let mut params = Params::default();
    let args = std::env::args().collect::<Vec<_>>();

    if let Some(cmd) = args.get(1..) {
        let len = cmd.len();
        let mut i = 0;

        while i < len {
            match cmd[i].as_str() {
                "-ic"            => { params.ic = parse(&cmd[i + 1])?; i += 1 }
                "-beta"          => { params.beta = parse(&cmd[i + 1])?; i += 1 },
                "-load1"         => { params.load1 = Some(parse(&cmd[i + 1])?); i += 1 },
                "-load2"         => { params.load2 = Some(parse(&cmd[i + 1])?); i += 1 },
                "-icut"          => { params.i_cutoff = parse(&cmd[i + 1])?; i += 1 },
                "-ocut"          => { params.o_cutoff = parse(&cmd[i + 1])?; i += 1 },
                "-lpass"         => { params.rc_lpass = parse(&cmd[i + 1])?; i += 1 },
                "-vcc"           => { params.vcc = parse(&cmd[i + 1])?; i += 1 },
                "-ve"            => { params.ve = parse(&cmd[i + 1])?; i += 1 },
                "-vbe"           => { params.vbe = parse(&cmd[i + 1])?; i += 1 },
                "-vbias"         => { params.vbias = Some(parse(&cmd[i + 1])?); i += 1 },
                other            => get_mode_and_bias(&mut params.mode, &mut params.bias, other)?,
            }
            i += 1;
        }
    }

    Ok(params)
}

fn rcf(rcf1: f64, rcf2: f64) -> f64 {
    1.0 / (PI2 * rcf1 * rcf2)
}

fn select(what: &str, suggest: f64, mut f: impl FnMut(f64)) -> Result<(), Error> {
    use core::str::FromStr;
    use std::io::{Read, Write};

    let mut stdin = std::io::stdin();

    loop {
        print!("{what} is: \x1B[1;33m{suggest:.2e}\x1B[0m. Pick your value: ");
        std::io::stdout().flush().unwrap();
        let mut buffer = [0u8; 32];
        match stdin.read(&mut buffer) {
            Ok(n) => {
                let slice = &buffer[..n];
                let sr = str::from_utf8(slice).map(str::trim);
                if sr.is_ok_and(|q| q == "quit") {
                    println!("--Aborted");
                    return Err(Error::Aborted);
                }
                if let Ok(Ok(new)) = sr.map(<f64 as FromStr>::from_str) {
                    f(new);
                    break;
                }
            },
            Err(_) => {}
        }
    }

    Ok(())
}

fn print_mode(mode: &BjtMode, bias: &BiasConfig) {
    println!("\x1B[1;32m{}\x1B[0m - \x1B[1;32m{}\x1B[0m", mode, bias);
}

fn analyze(params: &Params) -> Result<(), Error> {
    match params.mode {
        BjtMode::CommonEmitter => match params.bias {
            BiasConfig::VoltageDivider => voltage_divider_ce(params)?,
            BiasConfig::CollectorFeedback => collector_feedback(params)?,
            BiasConfig::EmitterBias => return Err(Error::DiscouragedMode),
        },
        BjtMode::CommonCollector => match params.bias {
            BiasConfig::VoltageDivider => voltage_divider_cc(params)?,
            BiasConfig::CollectorFeedback => return Err(Error::DiscouragedMode),
            BiasConfig::EmitterBias => emitter_bias_cc(params)?,
        },
    }

    Ok(())
}

const HELP: &'static str = r"
Usage:
    bjt <Commands> <value> <Mode> <Bias>

Commands
    -h, --help : print help
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

Mode (default: ce)
    -cc        : set the mode of the device to common collector
    -ce        : set the mode of the device to common emitter

Bias (default: voltdiv)
    -vdv       : set the bias configuration to voltage divider bias
    -feedback  : set the bias configuration to collectod feedback resistor
    -emb       : set the bias configuration to emitter bias

Example:
    bjt -ic 500e-6 -vcc 23 -ve 2 -vbe 0.65 -beta 493.2 -ce -vdv -vbias 9
";

fn main() {
    if let Err(err) = get_params().as_ref().map(analyze) {
        match err {
            Error::NeedsHelp => println!("{HELP}"),
            Error::UnableToParseFloat(e) |
            Error::InvalidArgs(e) => {
                println!("ERROR: {e}");
                println!("{HELP}");
            },
            _ => println!("{err}"),
        }
    }
}
