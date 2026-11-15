use std::io::Write;

use crate::rpar;
use crate::util::{Error, rcf, self};
use crate::parser::BjtParams;

pub(crate) const BOLTZMANN:   f64 = 1.380649e-23;
pub(crate) const CHARGE:      f64 = 1.602176634e-19;
pub(crate) const TEMPERATURE: f64 = 300.15;
pub(crate) const VT:          f64 = BOLTZMANN * TEMPERATURE / CHARGE;

pub(crate) struct ACNetwork {
    pub(crate) r_bypass: Option<f64>,
    pub(crate) c_bypass: Option<f64>,
    /// bypassed gain
    pub(crate) gain: f64,
}

impl core::fmt::Debug for ACNetwork {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if let Some(r_bypass) = self.r_bypass {
            write!(f, "r_bypass         : {r_bypass:.2e}\n")?;
        }
        if let Some(c_bypass) = self.c_bypass {
            write!(f, "c_bypass         : {c_bypass:.2e}\n")?;
        }
        write!(f, "ac gain          : {:.2}", self.gain)?;

        Ok(())
    }
}

pub(crate) fn analyze(params: &BjtParams) -> Result<(), Error> {
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

pub(crate) enum BjtMode {
    CommonEmitter,
    CommonCollector,
}

pub(crate) enum BiasConfig {
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
fn collector_feedback(params: &BjtParams) -> Result<(), Error> {
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

    params.clipping_threshold(vc);
    let rc_bypass = params.check_boost(zo, re, ree)?;

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
fn collector_feedback_divider(params: &BjtParams) -> Result<(), Error> {
    let gamma = params.gamma();
    let (re, ie, ib, ic) = params.emitter_design()?;

    let vb = params.vb();
    let mut rbg = 0.05 * params.beta * re;
    let max_rbg = vb / (ib * 10.0);

    util::select_minmax("rbg", &mut rbg, max_rbg)?;

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

    params.clipping_threshold(vc);
    let rc_bypass = params.check_boost(zo, re, ree)?;
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
fn split_collector_feedback(params: &BjtParams) -> Result<(), Error> {
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
    let rb = util::select_upto("Max rb", max_rb, max_rb)?;

    let v_split = vb + ib * rb;
    let max_rsg = v_split / (10.0 * ib);
    let rsg = util::select_upto("Max rsg", max_rsg, max_rsg)?;

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

    let shunt_freq = util::select_positive("Default shunted freq", 7.0)?;

    // Small-signal admittance at the split node S (Vcc is AC ground):
    // S -> Rsg -> ground
    // S -> Rb  -> base           (current vs / (rb + zb))
    // S -> Rsc -> collector -> Rc -> ground, where the collector is also driven by
    // the transistor current beta * vs / (rb + zb) flowing out of it.
    // Solving the collector node gives the three terms below.
    let y_shunt = 1.0 / rsg
        + (1.0 + params.beta * rc / (rc + rsc)) / (rb + zb)
        + 1.0 / (rc + rsc);
    let z_shunt = 1.0 / y_shunt;
    let shunt_cap = rcf(z_shunt, shunt_freq);

    params.clipping_threshold(vc);
    let rc_bypass = params.check_boost(zo, re, ree)?;

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

fn voltage_divider_ce(params: &BjtParams) -> Result<(), Error> {
    let gamma = params.gamma();
    let (re, ie, ib, ic) = params.emitter_design()?;

    let vb = params.vb();
    let (vc, rc) = params.vc_rc(ic, params.ve)?;

    let mut r2 = vb / (ib * 20.0);
    let max_r2 = 2.0 * r2;

    util::select_minmax("r2", &mut r2, max_r2)?;

    let ir2 = vb / r2;
    let ir1 = ib + ir2;
    let r1 = (params.v_bias() - vb) / ir1;
    let r_bias = rpar!(r1, r2);
    let ree = VT / ie;

    let rpi = gamma * ree;
    let zb = rpi + gamma * re;
    let zi = rpar!(zb, r_bias);
    let input_cap = rcf(zi, params.hpass);

    params.clipping_threshold(vc);
    let rc_bypass = params.check_boost(rc, re, ree)?;
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

fn bootstrap_ce(params: &BjtParams) -> Result<(), Error> {
    let gamma = params.gamma();
    let (re, ie, ib, ic) = params.emitter_design()?;

    let vb = params.vb();
    let v_bias = params.v_bias();

    let max_rb = (v_bias - vb) / (2.0 * ib);
    let mut rb = (0.1 * params.beta * re).min(max_rb);

    util::select_minmax("rb", &mut rb, max_rb)?;

    let vrb = rb * ib;
    let vr2 = vb + vrb;

    let mut r2 = vr2 / (20.0 * ib);
    let max_r2 = 2.0 * r2;

    util::select_minmax("r2", &mut r2, max_r2)?;

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

    params.clipping_threshold(vc);
    let rc_bypass = params.check_boost(rc, re_ac, ree)?;
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

fn voltage_divider_cc(params: &BjtParams) -> Result<(), Error> {
    let gamma = params.gamma();
    let (re, ie, ib, ic) = params.emitter_design()?;

    let vb = params.vb();

    let mut r2 = vb / (ib * 20.0);
    let max_r2 = 2.0 * r2;

    util::select_minmax("r2", &mut r2, max_r2)?;

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

fn fixed_emitter_bias_cc(params: &BjtParams) -> Result<(), Error> {
    let gamma = params.gamma();
    let (re, _, ib0, ic) = params.emitter_design()?;

    let vb0 = params.vb();
    let v_bias = params.v_bias();
    let rb = util::select_positive("Suggested rb", (v_bias - vb0) / ib0)?;
    let ib = (v_bias - params.vbe) / (rb + gamma * re);
    let ie = gamma * ib;
    let vb = ie * re + params.vbe;
    // vb = v_bias - ib * rb;

    let ree = VT / ie;
    let zb = gamma * (re + ree);
    let zi = rpar!(zb, rb);
    let zo = rpar!(re, ree);
    let input_cap = rcf(zi, params.hpass);

    print_mode(&params.mode, &params.bias);
    print!("ib               : {ib:.2e}\n");
    print!("ic               : {ic:.2e}\n");
    print!("vb               : {vb:.2e}\n");
    print!("-----------------------------------\n");
    print!("re               : {re:.2e}\n");
    print!("rb               : {rb:.2e}\n");
    print!("zi               : {zi:.2e}\n");
    print!("zo               : {zo:.2e}\n");
    print!("input cap        : {input_cap:.2e}\n");

    std::io::stdout().flush()?;

    Ok(())
}

fn bootstrap_cc(params: &BjtParams) -> Result<(), Error> {
    let gamma = params.gamma();
    let (re, ie, ib, ic) = params.emitter_design()?;

    let vb = params.vb();
    let v_bias = params.v_bias();

    let max_rb = (v_bias - vb) / (2.0 * ib);
    let mut rb = (0.05 * params.beta * re).min(max_rb);

    util::select_minmax("rb", &mut rb, max_rb)?;

    let vrb = rb * ib;
    let vr2 = vb + vrb;
    let mut r2 = vr2 / (20.0 * ib);
    let max_r2 = 2.0 * r2;

    util::select_minmax("r2", &mut r2, max_r2)?;

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
