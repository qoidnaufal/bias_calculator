use std::io::Write;

use crate::{Error, SelectError};
use crate::{rpar, rcf, parse, read_buffer};
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
            lpass:    15e3,
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
            BiasConfig::Bootstrap                   => bootstrap_ce(params)?,
            BiasConfig::FixedBias                   => return Err(Error::DiscouragedMode),
        },
        BjtMode::CommonCollector => match params.bias {
            BiasConfig::VoltageDivider              => voltage_divider_cc(params)?,
            BiasConfig::Bootstrap                   => bootstrap_cc(params)?,
            BiasConfig::FixedBias                   => fixed_emitter_bias_cc(params)?,
            BiasConfig::CollectorFeedback
            | BiasConfig::CollectorFeedbackDivider
            | BiasConfig::CollectorFeedbackSplit    => return Err(Error::DiscouragedMode),
        },
    }

    Ok(())
}

impl Params {
    fn vb(&self) -> f64 {
        self.ve + self.vbe
    }

    /// Sanity checks on the command line values, run once after parsing.
    fn validate(&self) -> Result<(), Error> {
        for (name, v) in [
            ("-ic", self.ic),
            ("-beta/-hfe", self.beta),
            ("-hpass", self.hpass),
            ("-lpass", self.lpass),
            ("-vcc", self.vcc),
            ("-ve", self.ve),
            ("-vbe", self.vbe),
        ] {
            if !(v.is_finite() && v > 0.0) {
                return Err(Error::InvalidArgs(format!("{name} must be a positive, finite number")));
            }
        }

        let vb = self.vb();

        if self.vcc <= vb {
            return Err(Error::InvalidArgs(format!(
                "-vcc ({:.2}) must be greater than Vb = Ve + Vbe ({vb:.2})", self.vcc
            )));
        }

        if let Some(vbias) = self.vbias {
            if !(vbias.is_finite() && vbias > vb) {
                return Err(Error::InvalidArgs(format!(
                    "-vbias ({vbias:.2}) must be greater than Vb = Ve + Vbe ({vb:.2})"
                )));
            }
        }

        if let Some(vc) = self.vc {
            if !(vc.is_finite() && vc > self.ve && vc < self.vcc) {
                return Err(Error::InvalidArgs(format!(
                    "-vc ({vc:.2}) must be between Ve ({:.2}) and Vcc ({:.2})", self.ve, self.vcc
                )));
            }
        }

        Ok(())
    }

    /// Lets the user pick `re` (the suggestion comes from the requested Ic and Ve),
    /// then returns the resulting (re, ie, ib, ic).
    fn emitter_design(&self) -> Result<(f64, f64, f64, f64), Error> {
        let ie0 = self.ic + self.ib();
        let re = select_positive("Suggested re", self.ve / ie0)?;

        let ie = self.ve / re;
        let ib = ie / self.gamma();
        let ic = ib * self.beta;

        Ok((re, ie, ib, ic))
    }

    /// `irc` is the DC current through Rc.
    /// `v_min` is the lowest acceptable collector voltage (strictly greater than it).
    /// Returns (vc, rc).
    fn vc_rc(&self, irc: f64, v_min: f64) -> Result<(f64, f64), Error> {
        let vc_target = match self.vc {
            Some(vc) => vc,
            None => {
                let mid = (self.vcc + self.ve) / 2.0;
                if mid > v_min { mid } else { (self.vcc + v_min) / 2.0 }
            }
        };

        if !(vc_target > v_min && vc_target < self.vcc) {
            return Err(Error::InvalidArgs(format!(
                "-vc must be between {v_min:.2} V and Vcc ({:.2} V) for this bias configuration", self.vcc
            )));
        }

        let suggested_rc = (self.vcc - vc_target) / irc;
        let mut rc = suggested_rc;

        select("Suggested rc", suggested_rc, |new| {
            if new.is_finite() && new > 0.0 && self.vcc - new * irc > v_min {
                rc = new;
                Ok(())
            } else {
                println!(
                    "rc must be positive and keep Vc above {v_min:.2} V (rc < {:.2e}).",
                    (self.vcc - v_min) / irc
                );
                Err(SelectError)
            }
        })?;

        let vc = self.vcc - rc * irc;

        if vc - self.ve < 1.0 {
            println!("Warning: Vce = {:.2} V is below 1 V, the transistor may saturate.", vc - self.ve);
        }

        Ok((vc, rc))
    }

    fn v_bias(&self) -> f64 {
        self.vbias.unwrap_or(self.vcc)
    }

    fn ib(&self) -> f64 {
        self.ic / self.beta
    }

    /// beta + 1
    fn gamma(&self) -> f64 {
        self.beta + 1.0
    }

    fn check_boost(&self) -> Result<Boost, Error> {
        match self.boost {
            1 => Ok(Boost::PlainBoost(prompt_positive("Set the \x1B[1;33mboosted frequency\x1B[0m: ")?)),
            2 => ValveParams::get(crate::valve::Mode::Dz).map(Boost::ValveBased),
            3 => ValveParams::get(crate::valve::Mode::Koren).map(Boost::ValveBased),
            _ => Ok(Boost::FlatGain),
        }
    }
}

enum Boost {
    ValveBased(ValveParams),
    PlainBoost(f64),
    FlatGain,
}

impl Boost {
    fn analyze(&self, zo: f64, re: f64, ree: f64) -> Result<crate::valve::ACNetwork, Error> {
        match self {
            Boost::FlatGain => {
                Ok(crate::valve::ACNetwork {
                    r_bypass: 0.0,
                    c_bypass: 0.0,
                    gain: zo / (re + ree),
                })
            }
            Boost::ValveBased(valve) => valve.ac_analysis(zo, re, ree),
            Boost::PlainBoost(hz) => {
                let ze = rpar!(re, ree);

                // Gain with r_bypass -> infinity (no bypass) and r_bypass -> 0 (full bypass).
                let min_gain = zo / (re + ree);
                let max_gain = zo / ree;

                let target = loop {
                    let t = prompt_positive(&format!(
                        "Unbypassed gain: \x1B[1;33m{min_gain:.2}\x1B[0m, set the desired bypass gain (between \x1B[1;33m{min_gain:.2}\x1B[0m and \x1B[1;33m{max_gain:.2}\x1B[0m): "
                    ))?;

                    if t > min_gain && t < max_gain {
                        break t;
                    }

                    println!("The desired gain must be strictly between {min_gain:.2} and {max_gain:.2}.");
                };

                // target = zo / ((re || r_bypass) + ree)  =>  re || r_bypass = zo / target - ree
                let a = zo / target - ree;
                let suggested_r_bypass = crate::par_extract(a, re);
                let mut stdin = std::io::stdin();

                loop {
                    let new_r = prompt_positive(&format!(
                        "Suggested r_bypass is \x1B[1;33m{suggested_r_bypass:.2e}\x1B[0m, select your value: "
                    ))?;

                    let new_c_bypass = rcf(new_r + ze, *hz);
                    let ac_re = rpar!(new_r, re);
                    let new_gain = zo / (ac_re + ree);

                    print!("c_bypass will be: \x1B[1;33m{new_c_bypass:.2e}\x1B[0m, ac_gain will be: \x1B[1;33m{new_gain:.2}\x1B[0m, continue? ");
                    std::io::stdout().flush()?;

                    let mut buffer = [0u8; 32];
                    let answer = read_buffer(&mut stdin, &mut buffer);

                    if answer.as_ref().is_ok_and(|q| *q == "quit" || *q == "q") {
                        println!("Aborted");
                        return Err(Error::Aborted);
                    }

                    if answer.is_ok_and(|y| y == "yes" || y == "y") {
                        return Ok(crate::valve::ACNetwork {
                            r_bypass: new_r,
                            c_bypass: new_c_bypass,
                            gain: new_gain,
                        });
                    }
                }
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
    Bootstrap,
    FixedBias,
}

fn print_mode(mode: &BjtMode, bias: &BiasConfig) {
    println!("\x1B[1;32m{}\x1B[0m - \x1B[1;32m{}\x1B[0m", mode, bias);
}

/// Only works in common emitter mode,
/// Vcc - Ibc * Rc - Ib * Rf - Vbe - Ie * Re = 0
fn collector_feedback(params: &Params) -> Result<(), Error> {
    let gamma = params.gamma();
    let (re, ie, ib, ic) = params.emitter_design()?;
    let ibc = ib + ic; // ibc = ie, this is the current through Rc

    let vb = params.vb();
    // Rf needs Vc > Vb.
    let (vc, rc) = params.vc_rc(ibc, vb)?;

    let rf = (vc - vb) / ib;
    let ree = VT / ie;

    let rpi = gamma * ree;
    let zb = rpi + gamma * re;
    let zi = zb * (rc + rf) / (rf + zb + gamma * rc);
    let zo = rpar!(rf, rc);

    let input_cap = rcf(zi, params.hpass);
    let low_pass_cap = rcf(zo, params.lpass);
    let rc_bypass = params.check_boost()?.analyze(zo, re, ree)?;

    print_mode(&params.mode, &params.bias);
    print!("vc               : {vc:.2}\n");
    print!("ic               : {ic:.2e}\n");
    print!("ibc              : {ibc:.2e}\n");
    print!("vce              : {:.2}\n", vc - params.ve);
    print!("-----------------------------------\n");
    print!("rc               : {rc:.2e}\n");
    print!("re               : {re:.2e}\n");
    print!("rf               : {rf:.2e}\n");
    print!("input cap        : {input_cap:.2e}\n");
    print!("low pass cap     : {low_pass_cap:.2e}\n");
    print!("{rc_bypass:?}\n");
    print!("-----------------------------------\n");
    print!("zi               : {zi:.2e}\n");
    print!("zo               : {zo:.2e}\n");
    print!("ree              : {ree:.2e}\n");
    print!("rpi              : {rpi:.2e}\n");
    print!("zb               : {zb:.2e}\n");

    std::io::stdout().flush()?;

    Ok(())
}

/// Only works in common emitter mode,
/// Vcc - (Irf + Ic) * Rc - Irf * Rf - Vb = 0, with Irf = Ib + Vb / Rbg
fn collector_feedback_divider(params: &Params) -> Result<(), Error> {
    let gamma = params.gamma();
    let (re, ie, ib, ic) = params.emitter_design()?;

    let vb = params.vb();
    let mut rbg = 0.1 * params.beta * re;
    let max_rbg = vb / (ib * 10.0);

    select_minmax("rbg", &mut rbg, max_rbg)?;

    let ir2 = vb / rbg;
    let irf = ib + ir2;
    let irc = irf + ic;

    // Rf needs Vc > Vb.
    let (vc, rc) = params.vc_rc(irc, vb)?;

    let rf = (vc - vb) / irf;
    let ree = VT / ie;

    let zb = gamma * (re + ree);
    let zi_feedback = zb * (rc + rf) / (rf + zb + gamma * rc);
    let zi = rpar!(zi_feedback, rbg);
    let zo = rpar!(rf, rc);

    let input_cap = rcf(zi, params.hpass);
    let rc_bypass = params.check_boost()?.analyze(zo, re, ree)?;
    let miller_cap = rcf(zi, params.lpass * (1.0 + rc_bypass.gain));

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
    print!("input cap        : {input_cap:.2e}\n");
    print!("miller cap       : {miller_cap:.2e}\n");
    print!("{rc_bypass:?}\n");
    print!("-----------------------------------\n");
    print!("zi               : {zi:.2e}\n");
    print!("zo               : {zo:.2e}\n");
    print!("ree              : {ree:.2e}\n");

    std::io::stdout().flush()?;

    Ok(())
}

/// Only works in common emitter mode,
fn split_collector_feedback(params: &Params) -> Result<(), Error> {
    let gamma = params.gamma();
    let (re, ie, ib, ic) = params.emitter_design()?;

    let vb = params.vb();
    let v_split_max = params.vcc / 2.0;
    let available_v_split = v_split_max - vb;

    if available_v_split <= 0.0 {
        return Err(Error::InvalidArgs(format!(
            "Vcc/2 ({v_split_max:.2} V) must be greater than Vb ({vb:.2} V) to place the split node"
        )));
    }

    let max_rb = available_v_split / ib;
    let rb = select_upto("Max rb", max_rb, max_rb)?;

    let v_split = vb + ib * rb;
    let max_rsg = v_split / (10.0 * ib);
    let rsg = select_upto("Max rsg", max_rsg, max_rsg)?;

    let irsg = v_split / rsg;
    let irsc = irsg + ib;
    let irc = irsc + ic;

    // Rsc needs Vc > Vsplit.
    let (vc, rc) = params.vc_rc(irc, v_split)?;
    let rsc = (vc - v_split) / irsc;

    let ree = VT / ie;
    let zb = gamma * (re + ree);
    let zi = rpar!(rb, zb);
    let zo = rpar!(rsc, rc);

    let input_cap = rcf(zi, params.hpass);
    let low_pass_cap = rcf(zo, params.lpass);

    let shunt_freq = select_positive("Default shunted freq", 7.0)?;

    // Small-signal admittance at the split node S (Vcc is AC ground):
    //   S -> Rsg -> ground
    //   S -> Rb -> base           (current vs / (rb + zb))
    //   S -> Rsc -> collector -> Rc -> ground, where the collector is also driven by
    //   the transistor current beta * vs / (rb + zb) flowing out of it.
    // Solving the collector node gives the three terms below.
    let y_shunt = 1.0 / rsg
        + (1.0 + params.beta * rc / (rc + rsc)) / (rb + zb)
        + 1.0 / (rc + rsc);
    let z_shunt = 1.0 / y_shunt;
    let shunt_cap = rcf(z_shunt, shunt_freq);

    let rc_bypass = params.check_boost()?.analyze(zo, re, ree)?;

    print_mode(&params.mode, &params.bias);
    print!("vc               : {vc:.2}\n");
    print!("ic               : {ic:.2e}\n");
    print!("irc              : {irc:.2e}\n");
    print!("irsc             : {irsc:.2e}\n");
    print!("vsplit           : {v_split:.2}\n");
    print!("vce              : {:.2}\n", vc - params.ve);
    print!("-----------------------------------\n");
    print!("rc               : {rc:.2e}\n");
    print!("re               : {re:.2e}\n");
    print!("rsg              : {rsg:.2e}\n");
    print!("rsc              : {rsc:.2e}\n");
    print!("rb               : {rb:.2e}\n");
    print!("input cap        : {input_cap:.2e}\n");
    print!("low pass cap     : {low_pass_cap:.2e}\n");
    print!("shunt cap        : {shunt_cap:.2e}\n");
    print!("{rc_bypass:?}\n");
    print!("-----------------------------------\n");
    print!("zi               : {zi:.2e}\n");
    print!("zo               : {zo:.2e}\n");
    print!("ree              : {ree:.2e}\n");

    std::io::stdout().flush()?;

    Ok(())
}

fn voltage_divider_ce(params: &Params) -> Result<(), Error> {
    let gamma = params.gamma();
    let (re, ie, ib, ic) = params.emitter_design()?;

    let vb = params.vb();
    let (vc, rc) = params.vc_rc(ic, params.ve)?;

    let mut r2 = vb / (ib * 20.0);
    let max_r2 = 2.0 * r2;

    select_minmax("r2", &mut r2, max_r2)?;

    let ir2 = vb / r2;
    let ir1 = ib + ir2;
    let r1 = (params.v_bias() - vb) / ir1;
    let r_bias = rpar!(r1, r2);
    let ree = VT / ie;

    let rpi = gamma * ree;
    let zb = rpi + gamma * re;
    let zi = rpar!(zb, r_bias);
    let input_cap = rcf(zi, params.hpass);

    let rc_bypass = params.check_boost()?.analyze(rc, re, ree)?;
    let miller_cap = rcf(zi, params.lpass * (1.0 + rc_bypass.gain));

    print_mode(&params.mode, &params.bias);
    print!("vc               : {vc:.2}\n");
    print!("vb               : {vb:.2}\n");
    print!("ic               : {ic:.2e}\n");
    print!("ib               : {ib:.2e}\n");
    print!("ir1              : {ir1:.2e}\n");
    print!("vce              : {:.2}\n", vc - params.ve);
    print!("-----------------------------------\n");
    print!("rc               : {rc:.2e}\n");
    print!("re               : {re:.2e}\n");
    print!("r1               : {r1:.2e}\n");
    print!("r2               : {r2:.2e}\n");
    print!("input cap        : {input_cap:.2e}\n");
    print!("miller cap       : {miller_cap:.2e}\n");
    print!("{rc_bypass:?}\n");
    print!("-----------------------------------\n");
    print!("zi               : {zi:.2e}\n");
    print!("zo               : {rc:.2e}\n");
    print!("ree              : {ree:.2e}\n");
    print!("rpi              : {rpi:.2e}\n");
    print!("zb               : {zb:.2e}\n");

    std::io::stdout().flush()?;

    Ok(())
}

fn bootstrap_ce(params: &Params) -> Result<(), Error> {
    let gamma = params.gamma();
    let (re, ie, ib, ic) = params.emitter_design()?;

    let vb = params.vb();
    let v_bias = params.v_bias();

    let max_rb = (v_bias - vb) / (2.0 * ib);
    let mut rb = (0.1 * params.beta * re).min(max_rb);

    select_minmax("rb", &mut rb, max_rb)?;

    let vrb = rb * ib;
    let vr2 = vb + vrb;

    let mut r2 = vr2 / (20.0 * ib);
    let max_r2 = 2.0 * r2;

    select_minmax("r2", &mut r2, max_r2)?;

    let ir2 = vr2 / r2;
    let ir1 = ib + ir2;
    let r1 = (v_bias - vr2) / ir1;
    let r_bias = rpar!(r1, r2);
    let ree = VT / ie;

    let re_ac = rpar!(re, r_bias);
    let rpi = gamma * ree;
    let zb = rpi + gamma * re_ac;
    let rb_eff = rb * (re_ac + ree) / ree;
    let zi = rpar!(zb, rb_eff);
    let input_cap = rcf(zi, params.hpass);
    let bootstrap_cap = rcf(r_bias, params.hpass);

    let (vc, rc) = params.vc_rc(ic, params.ve)?;

    let rc_bypass = params.check_boost()?.analyze(rc, re_ac, ree)?;
    let miller_cap = rcf(zi, params.lpass * (1.0 + rc_bypass.gain));

    print_mode(&params.mode, &params.bias);
    print!("vc               : {vc:.2}\n");
    print!("vb               : {vb:.2}\n");
    print!("ic               : {ic:.2e}\n");
    print!("ib               : {ib:.2e}\n");
    print!("ir1              : {ir1:.2e}\n");
    print!("vce              : {:.2}\n", vc - params.ve);
    print!("-----------------------------------\n");
    print!("rc               : {rc:.2e}\n");
    print!("re               : {re:.2e}\n");
    print!("r1               : {r1:.2e}\n");
    print!("r2               : {r2:.2e}\n");
    print!("rb               : {rb:.2e}\n");
    print!("input cap        : {input_cap:.2e}\n");
    print!("bootstrap cap    : {bootstrap_cap:.2e}\n");
    print!("miller cap       : {miller_cap:.2e}\n");
    print!("{rc_bypass:?}\n");
    print!("-----------------------------------\n");
    print!("zi               : {zi:.2e}\n");
    print!("zo               : {rc:.2e}\n");
    print!("ree              : {ree:.2e}\n");
    print!("rpi              : {rpi:.2e}\n");
    print!("zb               : {zb:.2e}\n");

    std::io::stdout().flush()?;

    Ok(())
}

fn voltage_divider_cc(params: &Params) -> Result<(), Error> {
    let gamma = params.gamma();
    let (re, ie, ib, ic) = params.emitter_design()?;

    let vb = params.vb();

    let mut r2 = vb / (ib * 20.0);
    let max_r2 = 2.0 * r2;

    select_minmax("r2", &mut r2, max_r2)?;

    let ir2 = vb / r2;
    let ir1 = ib + ir2;
    let r1 = (params.v_bias() - vb) / ir1;
    let r_bias = rpar!(r1, r2);
    let ree = VT / ie;

    let rpi = gamma * ree;
    let zb = rpi + gamma * re;
    let zi = rpar!(zb, r_bias);
    let zo = rpar!(re, ree);
    let input_cap = rcf(zi, params.hpass);

    print_mode(&params.mode, &params.bias);
    print!("vb               : {vb:.2e}\n");
    print!("ic               : {ic:.2e}\n");
    print!("ib               : {ib:.2e}\n");
    print!("ir1              : {ir1:.2e}\n");
    print!("-----------------------------------\n");
    print!("re               : {re:.2e}\n");
    print!("r1               : {r1:.2e}\n");
    print!("r2               : {r2:.2e}\n");
    print!("input cap        : {input_cap:.2e}\n");
    print!("-----------------------------------\n");
    print!("zi               : {zi:.2e}\n");
    print!("zo               : {zo:.2e}\n");
    print!("rpi              : {rpi:.2e}\n");
    print!("zb               : {zb:.2e}\n");
    std::io::stdout().flush()?;

    Ok(())
}

fn fixed_emitter_bias_cc(params: &Params) -> Result<(), Error> {
    let gamma = params.gamma();
    let (re, ie, ib, ic) = params.emitter_design()?;

    let vb = params.vb();
    let rb = select_positive("Suggested rb", (params.v_bias() - vb) / ib)?;

    let ree = VT / ie;
    let zb = gamma * (re + ree);
    let zi = rpar!(zb, rb);
    let zo = rpar!(re, ree);
    let input_cap = rcf(zi, params.hpass);

    print_mode(&params.mode, &params.bias);
    print!("ib               : {ib:.2e}\n");
    print!("ic               : {ic:.2e}\n");
    print!("-----------------------------------\n");
    print!("re               : {re:.2e}\n");
    print!("rb               : {rb:.2e}\n");
    print!("zi               : {zi:.2e}\n");
    print!("zo               : {zo:.2e}\n");
    print!("input cap        : {input_cap:.2e}\n");

    std::io::stdout().flush()?;

    Ok(())
}

fn bootstrap_cc(params: &Params) -> Result<(), Error> {
    let gamma = params.gamma();
    let (re, ie, ib, ic) = params.emitter_design()?;

    let vb = params.vb();
    let v_bias = params.v_bias();

    let max_rb = (v_bias - vb) / (2.0 * ib);
    let mut rb = (0.05 * params.beta * re).min(max_rb);

    select_minmax("rb", &mut rb, max_rb)?;

    let vrb = rb * ib;
    let vr2 = vb + vrb;
    let mut r2 = vr2 / (20.0 * ib);
    let max_r2 = 2.0 * r2;

    select_minmax("r2", &mut r2, max_r2)?;

    let ir2 = vr2 / r2;
    let ir1 = ib + ir2;
    let r1 = (v_bias - vr2) / ir1;
    let r_bias = rpar!(r1, r2);
    let ree = VT / ie;

    let re_ac = rpar!(re, r_bias);
    let rpi = gamma * ree;
    let zb = rpi + gamma * re_ac;
    let rb_eff = rb * (re_ac + ree) / ree;
    let zi = rpar!(zb, rb_eff);
    // The bias network also loads the emitter through the bootstrap cap.
    let zo = rpar!(re_ac, ree);
    let input_cap = rcf(zi, params.hpass);
    let bootstrap_cap = rcf(r_bias, params.hpass);

    print_mode(&params.mode, &params.bias);
    print!("ic               : {ic:.2e}\n");
    print!("ir1              : {ir1:.2e}\n");
    print!("-----------------------------------\n");
    print!("re               : {re:.2e}\n");
    print!("r1               : {r1:.2e}\n");
    print!("r2               : {r2:.2e}\n");
    print!("rb               : {rb:.2e}\n");
    print!("input cap        : {input_cap:.2e}\n");
    print!("bootstrap cap    : {bootstrap_cap:.2e}\n");
    print!("-----------------------------------\n");
    print!("zi               : {zi:.2e}\n");
    print!("zo               : {zo:.2e}\n");
    print!("rpi              : {rpi:.2e}\n");
    print!("zb               : {zb:.2e}\n");

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
            // Only the flags that take a value consume the next argument.
            match cmd[i].as_str() {
                "-ic"              => { params.ic = flag_value(cmd, i)?; i += 1 },
                "-beta" | "-hfe"   => { params.beta = flag_value(cmd, i)?; i += 1 },
                "-hpass"           => { params.hpass = flag_value(cmd, i)?; i += 1 },
                "-lpass"           => { params.lpass = flag_value(cmd, i)?; i += 1 },
                "-vcc"             => { params.vcc = flag_value(cmd, i)?; i += 1 },
                "-ve"              => { params.ve = flag_value(cmd, i)?; i += 1 },
                "-vc"              => { params.vc = Some(flag_value(cmd, i)?); i += 1 },
                "-vbe"             => { params.vbe = flag_value(cmd, i)?; i += 1 },
                "-vbias"           => { params.vbias = Some(flag_value(cmd, i)?); i += 1 },
                other              => get_mode_and_bias(&mut params, &mut mode_locked, &mut bias_locked, &mut boost_locked, other)?,
            }
            i += 1;
        }
    }

    params.validate()?;

    if matches!(params.mode, BjtMode::CommonCollector) {
        if params.vc.is_some() {
            eprintln!("Warning: -vc has no effect in common collector mode.");
        }
        if params.boost != 0 {
            eprintln!("Warning: boost options have no effect in common collector mode.");
        }
    }

    Ok(params)
}

fn flag_value(cmd: &[String], i: usize) -> Result<f64, Error> {
    let raw = cmd
        .get(i + 1)
        .ok_or_else(|| Error::InvalidArgs(format!("{} requires a value", cmd[i])))?;

    Ok(parse(raw)?)
}

fn lock_option(locked: &mut bool, group: &str, flag: &str) -> Result<(), Error> {
    if *locked {
        return Err(Error::InvalidArgs(format!("{flag}: only one {group} option may be given")));
    }
    *locked = true;
    Ok(())
}

fn get_mode_and_bias(params: &mut Params, mode_locked: &mut bool, bias_locked: &mut bool, boost_locked: &mut bool, s: &str) -> Result<(), Error> {
    match s {
        "-ce"            => { lock_option(mode_locked, "mode", s)?;  params.mode = BjtMode::CommonEmitter },
        "-cc"            => { lock_option(mode_locked, "mode", s)?;  params.mode = BjtMode::CommonCollector },
        "-vdiv"          => { lock_option(bias_locked, "bias", s)?;  params.bias = BiasConfig::VoltageDivider },
        "-cfb"           => { lock_option(bias_locked, "bias", s)?;  params.bias = BiasConfig::CollectorFeedback },
        "-cfdiv"         => { lock_option(bias_locked, "bias", s)?;  params.bias = BiasConfig::CollectorFeedbackDivider },
        "-split"         => { lock_option(bias_locked, "bias", s)?;  params.bias = BiasConfig::CollectorFeedbackSplit },
        "-fixed"         => { lock_option(bias_locked, "bias", s)?;  params.bias = BiasConfig::FixedBias },
        "-bts"           => { lock_option(bias_locked, "bias", s)?;  params.bias = BiasConfig::Bootstrap },
        "-boost"         => { lock_option(boost_locked, "boost", s)?; params.boost = 1 },
        "-dz"            => { lock_option(boost_locked, "boost", s)?; params.boost = 2 },
        "-koren"         => { lock_option(boost_locked, "boost", s)?; params.boost = 3 },
        "-h" | "--help"  => return Err(Error::NeedsHelp),
        _                => return Err(Error::InvalidArgs(s.to_owned()))
    }

    Ok(())
}

/// Asks for a strictly positive, finite number. `q` / `quit` aborts.
fn prompt_positive(msg: &str) -> Result<f64, Error> {
    let mut stdin = std::io::stdin();

    loop {
        print!("{msg}");
        std::io::stdout().flush()?;

        let mut buffer = [0u8; 32];
        let s = read_buffer(&mut stdin, &mut buffer);
        if s.as_ref().is_ok_and(|q| *q == "quit" || *q == "q") {
            println!("Aborted");
            return Err(Error::Aborted);
        }
        if let Ok(Ok(v)) = s.map(crate::parse) {
            if v.is_finite() && v > 0.0 {
                return Ok(v);
            }
        }
    }
}

pub(crate) fn select(what: &str, suggest: f64, mut f: impl FnMut(f64) -> Result<(), SelectError>) -> Result<(), Error> {
    let mut stdin = std::io::stdin();

    loop {
        print!("{what} is: \x1B[1;33m{suggest:.2e}\x1B[0m. Pick your value (type \x1B[1;32mquit\x1B[0m or \x1B[1;32mq\x1B[0m to abort): ");
        std::io::stdout().flush()?;

        let mut buffer = [0u8; 32];
        let s = read_buffer(&mut stdin, &mut buffer);
        if s.as_ref().is_ok_and(|q| *q == "quit" || *q == "q") {
            println!("Aborted");
            return Err(Error::Aborted);
        }
        if let Ok(Ok(new)) = s.map(crate::parse) {
            match f(new) {
                Ok(_) => break,
                Err(_err) => {}
            }
        }
    }

    Ok(())
}

/// Like `select`, but only accepts a positive, finite value.
pub(crate) fn select_positive(what: &str, suggest: f64) -> Result<f64, Error> {
    let mut value = suggest;

    select(what, suggest, |new| {
        if new.is_finite() && new > 0.0 {
            value = new;
            Ok(())
        } else {
            println!("The value must be a positive number.");
            Err(SelectError)
        }
    })?;

    Ok(value)
}

/// Like `select`, but only accepts 0 < value <= max.
pub(crate) fn select_upto(what: &str, suggest: f64, max: f64) -> Result<f64, Error> {
    let mut value = suggest;

    select(what, suggest, |new| {
        if new.is_finite() && new > 0.0 && new <= max {
            value = new;
            Ok(())
        } else {
            println!("The value must satisfy 0 < value <= {max:.2e}.");
            Err(SelectError)
        }
    })?;

    Ok(value)
}

pub(crate) fn select_minmax(what: &str, target: &mut f64, max: f64) -> Result<(), Error> {
    if !(max.is_finite() && max > 0.0) {
        return Err(Error::InvalidArgs(format!("No valid range for {what} with the given parameters")));
    }

    let mut stdin = std::io::stdin();

    loop {
        print!("Suggested {what} is: \x1B[1;33m{:.2e}\x1B[0m - \x1B[1;33m{max:.2e}\x1B[0m. Pick your value (type \x1B[1;32mquit\x1B[0m or \x1B[1;32mq\x1B[0m to abort): ", *target);
        std::io::stdout().flush()?;

        let mut buffer = [0u8; 32];
        let s = read_buffer(&mut stdin, &mut buffer);
        if s.as_ref().is_ok_and(|q| *q == "quit" || *q == "q") {
            println!("Aborted");
            return Err(Error::Aborted);
        }
        if let Ok(Ok(new)) = s.map(crate::parse) {
            if new.is_finite() && new > 0.0 && new <= max {
                *target = new;
                break
            }
            println!("The value must satisfy 0 < {what} <= {max:.2e}.");
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
            BiasConfig::Bootstrap                => "Bootstrap",
            BiasConfig::FixedBias                => "Fixed Bias",
        };
        write!(f, "{s}")
    }
}
