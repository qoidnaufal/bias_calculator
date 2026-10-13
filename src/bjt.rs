use std::io::{Read, Write};
use core::str::FromStr;

use crate::{Error, SelectError};
use crate::{rpar, rcf, parse};
use crate::valve::ValveParams;

const BOLTZMANN:   f64 = 1.380649e-23;
const CHARGE:      f64 = 1.602176634e-19;
const TEMPERATURE: f64 = 300.15;
const VT:          f64 = BOLTZMANN * TEMPERATURE / CHARGE;

// fn ac_gain(zo: f64, re: f64, ree: f64) -> f64 {
//     zo / (re + ree)
// }

pub(crate) struct Params {
    // load:     Option<f64>,
    ic:       f64,
    beta:     f64,
    hpass:    f64,
    lpass:    f64,
    vcc:      f64,
    vbe:      f64,
    ve:       f64,
    vbias:    Option<f64>,
    boost:    bool,
    mode:     BjtMode,
    bias:     BiasConfig,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            // load:     None,
            ic:       1e-3,
            beta:     1e2,
            hpass:    7.0,
            lpass:    20e3,
            vcc:      24.0,
            vbe:      0.65,
            ve:       2.0,
            vbias:    None,
            boost:    false,
            mode:     BjtMode::CommonEmitter,
            bias:     BiasConfig::VoltageDivider,
        }
    }
}

pub(crate) fn analyze(params: &Params) -> Result<(), Error> {
    match params.mode {
        BjtMode::CommonEmitter => match params.bias {
            BiasConfig::VoltageDivider              => voltage_divider_ce(params)?,
            BiasConfig::CollectorFeedback           => collector_feedback(params)?,
            BiasConfig::CollectorFeedbackDivider    => collector_feedback_divider(params)?,
            BiasConfig::EmitterBias                 => return Err(Error::DiscouragedMode),
        },
        BjtMode::CommonCollector => match params.bias {
            BiasConfig::VoltageDivider              => voltage_divider_cc(params)?,
            BiasConfig::CollectorFeedback
            | BiasConfig::CollectorFeedbackDivider  => return Err(Error::DiscouragedMode),
            BiasConfig::EmitterBias                 => emitter_bias_cc(params)?,
        },
    }

    Ok(())
}

impl Params {
    fn vb(&self) -> f64 {
        self.ve + self.vbe
    }

    fn vrc(&self) -> f64 {
        (self.vcc - self.ve) / 2.0
    }

    fn v_bias(&self) -> f64 {
        self.vbias.unwrap_or(self.vcc)
    }

    fn ib(&self) -> f64 {
        self.ic / self.beta
    }

    fn check_boost(&self) -> Option<Boost> {
        if self.boost {
            let mut stdin = std::io::stdin();

            loop {
                print!("Match with 12ax7 valve (y/n): ");
                std::io::stdout().flush().ok()?;

                let mut buffer = [0u8; 32];
                let n = stdin.read(&mut buffer).ok()?;
                if let Ok(s) = str::from_utf8(&buffer[..n]).map(str::trim) {
                    match s {
                        "q" | "quit" => {
                            println!("Aborted");
                            return None;
                        },
                        "y" | "yes" => return ValveParams::get().ok().map(Boost::ValveBased),
                        "n" | "no" => {
                            print!("Set the cutoff frequency: ");
                            std::io::stdout().flush().ok()?;
                            let mut buffer = [0u8; 32];
                            let n = stdin.read(&mut buffer).ok()?;
                            if let Ok(Ok(hz)) = str::from_utf8(&buffer[..n])
                                .map(str::trim)
                                .map(<f64 as FromStr>::from_str)
                            {
                                return Some(Boost::PlainBoost(hz));
                            }
                        },
                        _ => {},
                    }
                }
            }
        } else {
            None
        }
    }

    // fn rpi(&self) -> f64 {
    //     self.beta * (VT / self.ic)
    // }
}

enum Boost {
    ValveBased(ValveParams),
    PlainBoost(f64),
}

impl Boost {
    fn analyze(&self, rc: f64, re: f64, ree: f64) -> Result<crate::valve::ACNetwork, Error> {
        match self {
            Boost::ValveBased(valve) => valve.ac_analysis(rc, re, ree),
            Boost::PlainBoost(hz) => {
                let ze = rpar!(re, ree);
                let c_bypass = rcf(ze, *hz);
                Ok(crate::valve::ACNetwork {
                    r_bypass: 0.0,
                    c_bypass,
                })
            },
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
    CollectorFeedbackDivider,
    EmitterBias,
}

fn print_mode(mode: &BjtMode, bias: &BiasConfig) {
    println!("\x1B[1;32m{}\x1B[0m - \x1B[1;32m{}\x1B[0m", mode, bias);
}

/// Only works in common emitter mode,
/// Vcc - Ibc * Rc - Ib * Rf - Vbe - Ie * Re = 0
fn collector_feedback(params: &Params) -> Result<(), Error> {
    let mut ib = params.ib();
    let mut ie = params.ic + ib;
    let mut re = params.ve / ie;
    select("Suggested re", re, |new| Ok(re = new))?;
    ie = params.ve / re;
    ib = ie / (params.beta + 1.0);
    let ic = ib * params.beta;
    let ibc = ib + ic; // ibc = ie

    let vrc = params.vrc();
    let vb = params.vb();
    let vc = params.vcc - vrc;
    let rc = vrc / ibc;

    let rf = (vc - vb) / ib;
    let ree = VT / ie;

    let beta_inv = 1.0 / params.beta;
    let denom = beta_inv + (rc + re + ree) / rf;
    let zi = (re + ree) / denom;
    let zo = rpar!(rf, rc);

    let input_cap = rcf(zi, params.hpass);
    let low_pass_cap = rcf(zo, params.lpass);
    let rc_bypass = params.check_boost().map(|boost| boost.analyze(zo, re, ree));

    print_mode(&params.mode, &params.bias);
    print!("vc               : {vc:.2}\n");
    print!("ic               : {ic:.2e}\n");
    print!("ibc              : {ibc:.2e}\n");
    print!("vce              : {:.2}\n", vc - params.ve);
    print!("-----------------------------------\n");
    print!("rc               : {rc:.2e}\n");
    print!("re               : {re:.2e}\n");
    print!("rf               : {rf:.2e}\n");
    print!("zi               : {zi:.2e}\n");
    print!("zo               : {zo:.2e}\n");
    print!("input cap        : {input_cap:.2e}\n");
    print!("low pass cap     : {low_pass_cap:.2e}\n");

    if let Some(bypass) = rc_bypass {
        if let Ok(bypass) = bypass {
            print!("{bypass:?}\n");
        }
    }

    std::io::stdout().flush()?;

    Ok(())
}

/// Only works in common emitter mode,
/// Vcc - Ic * Rc - Irf * Rf - (Irf-Ib * R2) - Vbe - Ie * Re = 0
fn collector_feedback_divider(params: &Params) -> Result<(), Error> {
    let mut ib = params.ib();
    let mut ie = params.ic + ib;
    let mut re = params.ve / ie;
    select("Suggested re", re, |new| Ok(re = new))?;
    ie = params.ve / re;
    ib = ie / (params.beta + 1.0);
    let ic = ib * params.beta;

    let vrc = params.vrc();
    let vb = params.vb();
    let vc = params.vcc - vrc;

    let mut r2 = 0.1 * params.beta * re;
    select("Max r2", r2, |new| {
        if new > r2 {
            print!("New r2 exceeds maximum recommended value.\n");
            std::io::stdout().flush().unwrap();
            Err(SelectError)
        } else {
            r2 = new.min(r2);
            Ok(())
        }
    })?;

    let ir2 = vb / r2;
    let irf = ib + ir2;
    let rf = (vc - vb) / irf;
    let irc = irf + ic;
    let rc = vrc / irc;
    let ree = VT / ie;

    let beta_inv = 1.0 / params.beta;
    let denom = beta_inv + (rc + re + ree) / rf;
    let zi0 = (re + ree) / denom;
    let zi = rpar!(zi0, r2);
    let zo = rpar!(rf, rc);

    let input_cap = rcf(zi, params.hpass);
    let low_pass_cap = rcf(zo, params.lpass);
    let rc_bypass = params.check_boost().map(|boost| boost.analyze(zo, re, ree));

    print_mode(&params.mode, &params.bias);
    print!("vc               : {vc:.2}\n");
    print!("ic               : {ic:.2e}\n");
    print!("irc              : {irc:.2e}\n");
    print!("irf              : {irf:.2e}\n");
    print!("vce              : {:.2}\n", vc - params.ve);
    print!("-----------------------------------\n");
    print!("rc               : {rc:.2e}\n");
    print!("re               : {re:.2e}\n");
    print!("rf               : {rf:.2e}\n");
    print!("r2               : {r2:.2e}\n");
    print!("zi               : {zi:.2e}\n");
    print!("zo               : {zo:.2e}\n");
    print!("input cap        : {input_cap:.2e}\n");
    print!("low pass cap     : {low_pass_cap:.2e}\n");
    print!("ree              : {ree:.2e}\n");

    if let Some(bypass) = rc_bypass {
        if let Ok(bypass) = bypass {
            print!("{bypass:?}\n");
        }
    }

    std::io::stdout().flush()?;

    Ok(())
}

fn voltage_divider_ce(params: &Params) -> Result<(), Error> {
    let mut ib = params.ib();
    let mut ie = params.ic + ib;
    let mut re = params.ve / ie;
    select("Suggested re", re, |new| Ok(re = new))?;
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
            print!("New r2 exceeds maximum recommended value\n");
            std::io::stdout().flush().unwrap();
            Err(SelectError)
        } else {
            r2 = new.min(r2);
            Ok(())
        }
    })?;

    let ir2 = vb / r2;
    let ir1 = ib + ir2;
    let r1 = (params.v_bias() - vb) / ir1;
    let r_bias = rpar!(r1, r2);
    let ree = VT / ie;

    let zi = rpar!(params.beta * (re + ree), r_bias);
    let input_cap = rcf(zi, params.hpass);
    let low_pass_cap = rcf(rc, params.lpass);
    let rc_bypass = params.check_boost().map(|boost| boost.analyze(rc, re, ree));

    print_mode(&params.mode, &params.bias);
    print!("vc               : {vc:.2}\n");
    print!("ic               : {ic:.2e}\n");
    print!("ir1              : {ir1:.2e}\n");
    print!("vce              : {:.2}\n", vc - params.ve);
    print!("-----------------------------------\n");
    print!("rc               : {rc:.2e}\n");
    print!("re               : {re:.2e}\n");
    print!("r1               : {r1:.2e}\n");
    print!("r2               : {r2:.2e}\n");
    print!("zi               : {zi:.2e}\n");
    print!("zo               : {rc:.2e}\n");
    print!("input cap        : {input_cap:.2e}\n");
    print!("low pass cap     : {low_pass_cap:.2e}\n");
    print!("re || ree        : {:.2e}\n", rpar!(re, ree));

    if let Some(bypass) = rc_bypass {
        if let Ok(bypass) = bypass {
            print!("{bypass:?}\n");
        }
    }

    std::io::stdout().flush()?;

    Ok(())
}

fn voltage_divider_cc(params: &Params) -> Result<(), Error> {
    let mut ib = params.ib();
    let mut ie = params.ic + ib;
    let mut re = params.ve / ie;
    select("Suggested re", re, |new| Ok(re = new))?;
    ie = params.ve / re;
    ib = ie / (params.beta + 1.0);
    let ic = ib * params.beta;

    let vb = params.vb();

    let mut r2 = 0.1 * params.beta * re;
    select("Max r2", r2, |new| {
        if new > r2 {
            print!("New r2 exceeds maximum recommended value\n");
            std::io::stdout().flush().unwrap();
            Err(SelectError)
        } else {
            r2 = new.min(r2);
            Ok(())
        }
    })?;
    let ir2 = vb / r2;
    let ir1 = ib + ir2;
    let r1 = (params.v_bias() - vb) / ir1;
    let r_bias = rpar!(r1, r2);
    let ree = VT / ie;

    let zb = (params.beta + 1.0) * (re + ree);
    let zi = rpar!(zb, r_bias);
    let zo = rpar!(re, ree);
    let input_cap = rcf(zi, params.hpass);

    print_mode(&params.mode, &params.bias);
    print!("ic               : {ic:.2e}\n");
    print!("ir1              : {ir1:.2e}\n");
    print!("-----------------------------------\n");
    print!("re               : {re:.2e}\n");
    print!("r1               : {r1:.2e}\n");
    print!("r2               : {r2:.2e}\n");
    print!("zi               : {zi:.2e}\n");
    print!("zo               : {zo:.2e}\n");
    print!("input cap        : {input_cap:.2e}\n");
    std::io::stdout().flush()?;

    Ok(())
}

fn emitter_bias_cc(params: &Params) -> Result<(), Error> {
    let mut ib = params.ib();
    let mut ie = params.ic + ib;
    let mut re = params.ve / ie;
    select("Suggested re", re, |new| Ok(re = new))?;
    ie = params.ve / re;
    ib = ie / (params.beta + 1.0);

    let vb = params.vb();

    let mut rb = (params.v_bias() - vb) / ib;
    select("Suggested rb", rb, |new| Ok(rb = new))?;
    let ree = VT / ie;

    // let zb = params.beta * ree + (params.beta + 1.0) * re;
    let zb = (params.beta + 1.0) * (re + ree);
    let zi = rpar!(zb, rb);
    let zo = rpar!(re, ree);
    let input_cap = rcf(zi, params.hpass);

    print_mode(&params.mode, &params.bias);
    print!("ib               : {ib:.2e}\n");
    print!("ic               : {:.2e}\n", ib * params.beta);
    print!("-----------------------------------\n");
    print!("re               : {re:.2e}\n");
    print!("rb               : {rb:.2e}\n");
    print!("zi               : {zi:.2e}\n");
    print!("zo               : {zo:.2e}\n");
    print!("input cap        : {input_cap:.2e}\n");
    std::io::stdout().flush()?;

    Ok(())
}

pub(crate) fn get_params() -> Result<Params, Error> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() <= 1 { return Err(Error::NeedsHelp) }

    let mut params = Params::default();

    if let Some(cmd) = args.get(1..) {
        let len = cmd.len();
        let mut i = 0;

        let beta = String::from("-beta");
        let hfe = String::from("-hfe");
        if cmd.contains(&beta) && cmd.contains(&hfe) {
            return Err(Error::InvalidArgs("Both beta & hfe are defined at the same time, use only one of them.".to_owned()))
        }

        while i < len {
            match cmd[i].as_str() {
                "-ic"              => { params.ic = parse(&cmd[i + 1])?; i += 1 }
                "-beta" | "-hfe"   => { params.beta = parse(&cmd[i + 1])?; i += 1 },
                // "-load"            => { params.load = Some(parse(&cmd[i + 1])?); i += 1 },
                "-hpass"           => { params.hpass = parse(&cmd[i + 1])?; i += 1 },
                "-lpass"           => { params.lpass = parse(&cmd[i + 1])?; i += 1 },
                "-vcc"             => { params.vcc = parse(&cmd[i + 1])?; i += 1 },
                "-ve"              => { params.ve = parse(&cmd[i + 1])?; i += 1 },
                "-vbe"             => { params.vbe = parse(&cmd[i + 1])?; i += 1 },
                "-vbias"           => { params.vbias = Some(parse(&cmd[i + 1])?); i += 1 },
                other              => get_mode_and_bias(&mut params, other)?,
            }
            i += 1;
        }
    }

    Ok(params)
}

fn get_mode_and_bias(params: &mut Params, s: &str) -> Result<(), Error> {
    match s {
        "-ce"            => params.mode = BjtMode::CommonEmitter,
        "-cc"            => params.mode = BjtMode::CommonCollector,
        "-vdiv"          => params.bias = BiasConfig::VoltageDivider,
        "-cfb"           => params.bias = BiasConfig::CollectorFeedback,
        "-cfdiv"         => params.bias = BiasConfig::CollectorFeedbackDivider,
        "-emb"           => params.bias = BiasConfig::EmitterBias,
        "-boost"         => params.boost = true,
        "-h" | "--help"  => return Err(Error::NeedsHelp),
        _                => return Err(Error::InvalidArgs(s.to_owned()))
    }

    Ok(())
}

pub(crate) fn select(what: &str, suggest: f64, mut f: impl FnMut(f64) -> Result<(), SelectError>) -> Result<(), Error> {
    let mut stdin = std::io::stdin();

    loop {
        print!("{what} is: \x1B[1;33m{suggest:.2e}\x1B[0m. Pick your value (type \x1B[1;32mquit\x1B[0m or \x1B[1;32mq\x1B[0m to abort): ");
        std::io::stdout().flush()?;

        let mut buffer = [0u8; 32];
        let n = stdin.read(&mut buffer)?;
        let s = str::from_utf8(&buffer[..n]).map(str::trim);
        if s.is_ok_and(|q| q == "quit" || q == "q") {
            println!("Aborted");
            return Err(Error::Aborted);
        }
        if let Ok(Ok(new)) = s.map(<f64 as FromStr>::from_str) {
            match f(new) {
                Ok(_) => break,
                Err(_err) => {}
            }
        }
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
            BiasConfig::VoltageDivider           => "Voltage Divider",
            BiasConfig::CollectorFeedback        => "Collector Feedback",
            BiasConfig::CollectorFeedbackDivider => "Collector Feedback Divider",
            BiasConfig::EmitterBias              => "Emitter Bias",
        };
        write!(f, "{s}")
    }
}
