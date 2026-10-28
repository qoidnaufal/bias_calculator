use std::io::{Read, Write};
use core::str::FromStr;

use crate::{Error, SelectError};
use crate::{rpar, rcf, parse};
use crate::valve::ValveParams;

pub(crate) const BOLTZMANN:   f64 = 1.380649e-23;
pub(crate) const CHARGE:      f64 = 1.602176634e-19;
pub(crate) const TEMPERATURE: f64 = 300.15;
pub(crate) const VT:          f64 = BOLTZMANN * TEMPERATURE / CHARGE;

pub(crate) struct Params {
    ic:       f64,
    beta:     f64,
    hpass:    f64,
    lpass:    f64,
    vcc:      f64,
    vbe:      f64,
    ve:       f64,
    vc:       Option<f64>,
    vbias:    Option<f64>,
    boost:    u8,
    mode:     BjtMode,
    bias:     BiasConfig,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            ic:       1e-3,
            beta:     1e2,
            hpass:    7.0,
            lpass:    20e3,
            vcc:      24.0,
            vbe:      0.65,
            vc:       None,
            ve:       2.0,
            vbias:    None,
            boost:    0,
            mode:     BjtMode::CommonEmitter,
            bias:     BiasConfig::VoltageDivider,
        }
    }
}

pub(crate) fn analyze(params: &Params) -> Result<(), Error> {
    match params.mode {
        BjtMode::CommonEmitter   => match params.bias {
            BiasConfig::VoltageDivider              => voltage_divider_ce(params)?,
            BiasConfig::CollectorFeedback           => collector_feedback(params)?,
            BiasConfig::CollectorFeedbackDivider    => collector_feedback_divider(params)?,
            BiasConfig::CollectorFeedbackSplit      => split_collector_feedback(params)?,
            BiasConfig::EmitterBias                 => return Err(Error::DiscouragedMode),
        },
        BjtMode::CommonCollector => match params.bias {
            BiasConfig::VoltageDivider              => voltage_divider_cc(params)?,
            BiasConfig::CollectorFeedback
            | BiasConfig::CollectorFeedbackDivider
            | BiasConfig::CollectorFeedbackSplit    => return Err(Error::DiscouragedMode),
            BiasConfig::EmitterBias                 => emitter_bias_cc(params)?,
        },
    }

    Ok(())
}

impl Params {
    fn vb(&self) -> f64 {
        self.ve + self.vbe
    }

    fn vc_vrc(&self) -> (f64, f64) {
        if let Some(vc) = self.vc {
            (vc, self.vcc - vc)
        } else {
            let vrc = (self.vcc - self.ve) / 2.0;
            let vc = self.vcc - vrc;
            (vc, vrc)
        }
    }

    fn v_bias(&self) -> f64 {
        self.vbias.unwrap_or(self.vcc)
    }

    fn ib(&self) -> f64 {
        self.ic / self.beta
    }

    fn alpha(&self) -> f64 {
        self.beta + 1.0
    }

    fn check_boost(&self) -> Option<Boost> {
        match self.boost {
            1 => {
                let mut stdin = std::io::stdin();
                loop {
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
                }
            },
            2 => return ValveParams::get(crate::valve::Mode::Dz).ok().map(Boost::ValveBased),
            3 => return ValveParams::get(crate::valve::Mode::Koren).ok().map(Boost::ValveBased),
            _ => return None
        }
    }
}

enum Boost {
    ValveBased(ValveParams),
    PlainBoost(f64),
}

impl Boost {
    fn analyze(&self, zo: f64, re: f64, ree: f64) -> Result<crate::valve::ACNetwork, Error> {
        match self {
            Boost::ValveBased(valve) => valve.ac_analysis(zo, re, ree),
            Boost::PlainBoost(hz) => {
                let ze = rpar!(re, ree);
                let c_bypass = rcf(ze, *hz);
                Ok(crate::valve::ACNetwork {
                    r_bypass: 0.0,
                    c_bypass,
                    gain: zo / ree,
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
    CollectorFeedbackSplit,
    EmitterBias,
}

fn print_mode(mode: &BjtMode, bias: &BiasConfig) {
    println!("\x1B[1;32m{}\x1B[0m - \x1B[1;32m{}\x1B[0m", mode, bias);
}

// /// ro = vaf / ic
// fn zi_cf(beta: f64, rc: f64, rf: f64, re: f64, ree: f64, ro: f64) -> f64 {
//     let b  = beta + 1.0;
//     let zb = b * (re + ree);

//     let n0 = rc*re*rf + b*ree*(rc*re + rc*rf + re*rf);
//     let d0 = rf*(rc + re) + b*ree*(rc + re);
//     let n1 = zb * (rc + rf);
//     let d1 = rf + b*(rc + re + ree);

//     (ro*n1 + n0) / (ro*d1 + d0)
// }

/// Only works in common emitter mode,
/// Vcc - Ibc * Rc - Ib * Rf - Vbe - Ie * Re = 0
fn collector_feedback(params: &Params) -> Result<(), Error> {
    let alpha = params.alpha();
    let mut ib = params.ib();
    let mut ie = params.ic + ib;
    let mut re = params.ve / ie;
    select("Suggested re", re, |new| Ok(re = new))?;
    ie = params.ve / re;
    ib = ie / (params.beta + 1.0);
    let ic = ib * params.beta;
    let ibc = ib + ic; // ibc = ie

    let (vc, vrc) = params.vc_vrc();
    let vb = params.vb();
    let rc = vrc / ibc;

    let rf = (vc - vb) / ib;
    let ree = VT / ie;
    // let ro = params.vaf / ic;

    let zb = alpha * (re + ree);
    let zi = zb * (rc + rf) / (rf + zb + alpha * rc);
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
    print!("ree              : {ree:.2e}\n");

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
    let alpha = params.alpha();
    let mut ib = params.ib();
    let mut ie = params.ic + ib;
    let mut re = params.ve / ie;
    select("Suggested re", re, |new| Ok(re = new))?;
    ie = params.ve / re;
    ib = ie / alpha;
    let ic = ib * params.beta;

    let (vc, vrc) = params.vc_vrc();
    let vb = params.vb();

    let mut rbg = 0.1 * params.beta * re;
    select("Max rbg", rbg, |new| {
        if new > rbg {
            print!("New rbg exceeds maximum recommended value.\n");
            std::io::stdout().flush().unwrap();
            Err(SelectError)
        } else {
            rbg = new;
            Ok(())
        }
    })?;

    let ir2 = vb / rbg;
    let irf = ib + ir2;
    let rf = (vc - vb) / irf;
    let irc = irf + ic;
    let rc = vrc / irc;
    let ree = VT / ie;

    let zb = alpha * (re + ree);
    let zi_feedback = zb * (rc + rf) / (rf + zb + alpha * rc);
    let zi = rpar!(zi_feedback, rbg);
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
    print!("rbg              : {rbg:.2e}\n");
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

/// Only works in common emitter mode,
fn split_collector_feedback(params: &Params) -> Result<(), Error> {
    let alpha = params.alpha();
    let mut ib = params.ib();
    let mut ie = params.ic + ib;
    let mut re = params.ve / ie;
    select("Suggested re", re, |new| Ok(re = new))?;
    ie = params.ve / re;
    ib = ie / alpha;

    let (vc, vrc) = params.vc_vrc();
    let vb = params.vb();

    let min_v_split = (vc - vb) / 2.0;
    if min_v_split <= vb {
        return Err(Error::Other(format!("Voltage at split node ({min_v_split:.2}) <= expected Vb ({vb:.2}). Readjust your params.")));
    }

    let mut rb = min_v_split / ib;
    select("Max rb", rb, |new| {
        if new > rb {
            print!("New rb exceeds the recommended value.\n");
            std::io::stdout().flush().unwrap();
            Err(SelectError)
        } else {
            rb = new;
            Ok(())
        }
    })?;

    let v_split = ib.mul_add(rb, vb);
    let mut rfg = v_split / (10.0 * ib);
    select("Max rfg", rfg, |new| {
        if new > rfg {
            print!("New rfg exceeds the recommended value.\n");
            std::io::stdout().flush().unwrap();
            Err(SelectError)
        } else {
            rfg = new;
            Ok(())
        }
    })?;

    let ic = ib * params.beta;
    let irfg = v_split / rfg;
    let irfc = irfg + ib;
    let rfc = (vc - v_split) / irfc;
    let irc = irfc + ic;
    let rc = vrc / irc;

    let ree = VT / ie;
    let zb = alpha * (re + ree);
    let zi = rpar!(rb, zb);
    let zo = rpar!(rfc, rc);

    let input_cap = rcf(zi, params.hpass);
    let low_pass_cap = rcf(zo, params.lpass);

    let mut shunt_freq = 7.0;
    select("Default shunted freq", shunt_freq, |new| Ok(shunt_freq = new))?;
    let z_shunt = rpar!(rfg, rfc, (rb + zb));
    let shunt_cap = rcf(z_shunt, shunt_freq);

    let rc_bypass = params.check_boost().map(|boost| boost.analyze(zo, re, ree));

    print_mode(&params.mode, &params.bias);
    print!("vc               : {vc:.2}\n");
    print!("ic               : {ic:.2e}\n");
    print!("irc              : {irc:.2e}\n");
    print!("irfc             : {irfc:.2e}\n");
    print!("vce              : {:.2}\n", vc - params.ve);
    print!("-----------------------------------\n");
    print!("rc               : {rc:.2e}\n");
    print!("re               : {re:.2e}\n");
    print!("rfg              : {rfg:.2e}\n");
    print!("rfc              : {rfc:.2e}\n");
    print!("rb               : {rb:.2e}\n");
    print!("zi               : {zi:.2e}\n");
    print!("zo               : {zo:.2e}\n");
    print!("input cap        : {input_cap:.2e}\n");
    print!("low pass cap     : {low_pass_cap:.2e}\n");
    print!("shunt cap        : {shunt_cap:.2e}\n");
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
    let (vc, vrc) = params.vc_vrc();
    let rc = vrc / ic;

    let mut r2 = 0.1 * params.beta * re;
    select("Max r2", r2, |new| {
        if new > r2 {
            print!("New r2 exceeds maximum recommended value\n");
            std::io::stdout().flush().unwrap();
            Err(SelectError)
        } else {
            r2 = new;
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
    print!("ree              : {ree:.2e}\n");

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
            r2 = new;
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

        let beta = String::from("-beta");
        let hfe = String::from("-hfe");
        if cmd.contains(&beta) && cmd.contains(&hfe) {
            return Err(Error::InvalidArgs("Both beta & hfe are defined at the same time, use only one of them.".to_owned()))
        }

        let mut mode_locked = false;
        let mut bias_locked = false;
        let mut boost_locked = false;
        let mut i = 0;

        while i < len {
            match cmd[i].as_str() {
                "-ic"              => { params.ic = parse(&cmd[i + 1])?; i += 1 },
                "-beta" | "-hfe"   => { params.beta = parse(&cmd[i + 1])?; i += 1 },
                "-hpass"           => { params.hpass = parse(&cmd[i + 1])?; i += 1 },
                "-lpass"           => { params.lpass = parse(&cmd[i + 1])?; i += 1 },
                "-vcc"             => { params.vcc = parse(&cmd[i + 1])?; i += 1 },
                "-ve"              => { params.ve = parse(&cmd[i + 1])?; i += 1 },
                "-vc"              => { params.vc = Some(parse(&cmd[i + 1])?); i += 1 },
                "-vbe"             => { params.vbe = parse(&cmd[i + 1])?; i += 1 },
                "-vbias"           => { params.vbias = Some(parse(&cmd[i + 1])?); i += 1 },
                other              => get_mode_and_bias(&mut params, &mut mode_locked, &mut bias_locked, &mut boost_locked, other)?,
            }
            i += 1;
        }
    }

    Ok(params)
}

fn get_mode_and_bias(params: &mut Params, mode_locked: &mut bool, bias_locked: &mut bool, boost_locked: &mut bool, s: &str) -> Result<(), Error> {
    match s {
        "-ce"            => if !*mode_locked { params.mode = BjtMode::CommonEmitter; *mode_locked = true },
        "-cc"            => if !*mode_locked { params.mode = BjtMode::CommonCollector; *mode_locked = true },
        "-vdiv"          => if !*bias_locked { params.bias = BiasConfig::VoltageDivider; *bias_locked = true },
        "-cfb"           => if !*bias_locked { params.bias = BiasConfig::CollectorFeedback; *bias_locked = true },
        "-cfdiv"         => if !*bias_locked { params.bias = BiasConfig::CollectorFeedbackDivider; *bias_locked = true },
        "-split"         => if !*bias_locked { params.bias = BiasConfig::CollectorFeedbackSplit; *bias_locked = true },
        "-emb"           => if !*bias_locked { params.bias = BiasConfig::EmitterBias; *bias_locked = true },
        "-boost"         => if !*boost_locked { params.boost = 1; *boost_locked = true },
        "-dz"            => if !*boost_locked { params.boost = 2; *boost_locked = true },
        "-koren"         => if !*boost_locked { params.boost = 3; *boost_locked = true },
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
        if let Ok(Ok(new)) = s.map(parse) {
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
            BiasConfig::CollectorFeedbackSplit   => "Collector Feedback Split",
            BiasConfig::EmitterBias              => "Emitter Bias",
        };
        write!(f, "{s}")
    }
}
