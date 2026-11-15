use crate::util::{Error, SelectError, self};
use crate::bjt::{BjtMode, BiasConfig, ACNetwork};
use crate::rpar;
use crate::valve::ValveMode;

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

pub(crate) enum Params {
    Bjt(BjtParams),
    Valve(ValveParams),
}

impl Params {
    pub(crate) fn analyze(&self) -> Result<(), Error> {
        match self {
            Params::Bjt(params) => crate::bjt::analyze(params),
            Params::Valve(params) => params.ac_analysis(),
        }
    }
}

pub(crate) struct BjtParams {
    pub(crate) ic:       f64,
    pub(crate) beta:     f64,
    pub(crate) hpass:    f64,
    pub(crate) lpass:    f64,
    pub(crate) vcc:      f64,
    pub(crate) vbe:      f64,
    pub(crate) ve:       f64,
    pub(crate) vc:       Option<f64>,
    pub(crate) vbias:    Option<f64>,
    pub(crate) boost:    bool,
    pub(crate) mode:     BjtMode,
    pub(crate) bias:     BiasConfig,
}

impl Default for BjtParams {
    fn default() -> Self {
        Self {
            ic:       500e-6,
            beta:     1e2,
            hpass:    3.3,
            lpass:    7.7e3,
            vcc:      24.0,
            vbe:      0.65,
            vc:       None,
            ve:       2.0,
            vbias:    None,
            boost:    false,
            mode:     BjtMode::CommonEmitter,
            bias:     BiasConfig::VoltageDivider,
        }
    }
}

impl BjtParams {
    pub(crate) fn new(args: Vec<String>) -> Result<Self, Error> {
        let mut params = Self::default();

        if let Some(cmd) = args.get(1..) {
            let len = cmd.len();

            let beta = String::from("-beta");
            let hfe = String::from("-hfe");
            if cmd.contains(&beta) && cmd.contains(&hfe) {
                return Err(Error::InvalidArgs("Both beta & hfe are defined at the same time, use only one of them.".to_owned()))
            }

            let mut mode_locked = false;
            let mut bias_locked = false;
            let mut i = 0;

            while i < len {
                match cmd[i].as_str() {
                    "-bjt"             => {},
                    "-ic"              => { params.ic = flag_value(cmd, i)?; i += 1 },
                    "-beta" | "-hfe"   => { params.beta = flag_value(cmd, i)?; i += 1 },
                    "-hpass"           => { params.hpass = flag_value(cmd, i)?; i += 1 },
                    "-lpass"           => { params.lpass = flag_value(cmd, i)?; i += 1 },
                    "-vcc"             => { params.vcc = flag_value(cmd, i)?; i += 1 },
                    "-ve"              => { params.ve = flag_value(cmd, i)?; i += 1 },
                    "-vc"              => { params.vc = Some(flag_value(cmd, i)?); i += 1 },
                    "-vbe"             => { params.vbe = flag_value(cmd, i)?; i += 1 },
                    "-vbias"           => { params.vbias = Some(flag_value(cmd, i)?); i += 1 },
                    other              => get_mode_and_bias(&mut params, &mut mode_locked, &mut bias_locked, other)?,
                }
                i += 1;
            }
        }

        params.validate()?;

        if matches!(params.mode, BjtMode::CommonCollector) {
            if params.vc.is_some() {
                eprintln!("Warning: -vc has no effect in common collector mode.");
            }
            if params.boost {
                eprintln!("Warning: boost options have no effect in common collector mode.");
            }
        }

        Ok(params)
    }

    pub(crate) fn vb(&self) -> f64 {
        self.ve + self.vbe
    }

    /// Sanity checks on the command line values, run once after parsing.
    pub(crate) fn validate(&self) -> Result<(), Error> {
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
    pub(crate) fn emitter_design(&self) -> Result<(f64, f64, f64, f64), Error> {
        let ie0 = self.ic + self.ib();
        let re = util::select_positive("Suggested re", self.ve / ie0)?;

        let ie = self.ve / re;
        let ib = ie / self.gamma();
        let ic = ib * self.beta;

        Ok((re, ie, ib, ic))
    }

    /// `irc` is the DC current through Rc.
    /// `v_min` is the lowest acceptable collector voltage (strictly greater than it).
    /// Returns (vc, rc).
    pub(crate) fn vc_rc(&self, irc: f64, v_min: f64) -> Result<(f64, f64), Error> {
        let vc_target = match self.vc {
            Some(vc) => vc,
            None => {
                let mid = self.vcc / 2.0 + self.ve;
                if mid > v_min && mid < self.vcc { mid } else { (self.vcc + v_min) / 2.0 }
            }
        };

        if !(vc_target > v_min && vc_target < self.vcc) {
            return Err(Error::InvalidArgs(format!(
                "-vc must be between {v_min:.2} V and Vcc ({:.2} V) for this bias configuration", self.vcc
            )));
        }

        let suggested_rc = (self.vcc - vc_target) / irc;
        let mut rc = suggested_rc;

        util::select("Suggested rc", suggested_rc, |new| {
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
            println!("[WARNING]: Vce = {:.2} V is below 1 V, the transistor may saturate.", vc - self.ve);
        }

        Ok((vc, rc))
    }

    pub(crate) fn v_bias(&self) -> f64 {
        self.vbias.unwrap_or(self.vcc)
    }

    pub(crate) fn ib(&self) -> f64 {
        self.ic / self.beta
    }

    /// beta + 1
    pub(crate) fn gamma(&self) -> f64 {
        self.beta + 1.0
    }

    pub(crate) fn clipping_threshold(&self, vc: f64) {
        let lo = vc - self.ve * 2.0;
        let hi = self.vcc - vc;
        println!("clipping threshold : lo = {lo:.2e} | hi = {hi:.2e}");
    }

    pub(crate) fn check_boost(&self, zo: f64, re: f64, ree: f64) -> Result<ACNetwork, Error> {
        let min_gain = zo / (re + ree);

        if self.boost {
            let hz = util::prompt_positive("Select boosted frequency: ")?;

            let ze = rpar!(re, ree);

            let max_gain = zo / ree;

            let target = loop {
                let t = util::prompt_positive(&format!(
                    "Available gain: \x1B[1;33m{min_gain:.2}\x1B[0m - \x1B[1;33m{max_gain:.2}\x1B[0m, set the desired gain: "
                ))?;

                let t = t.min(max_gain);
                if t > min_gain {
                    break t;
                }
            };

            if (target - max_gain).abs() < 1e-2 {
                return Ok(ACNetwork {
                    r_bypass: None,
                    c_bypass: Some(util::rcf(ze, hz)),
                    gain: max_gain,
                });
            }

            let a = zo / target - ree;
            let suggested_r_bypass = util::par_extract(a, re);

            let new_r = util::prompt_positive(&format!(
                "Suggested r_bypass is \x1B[1;33m{suggested_r_bypass:.2e}\x1B[0m, select your value: "
            ))?;

            let suggested_c_bypass = util::rcf(new_r + ze, hz);
            let ac_re = rpar!(new_r, re);

            let new_gain = zo / (ac_re + ree);
            println!("ac_gain is: \x1B[1;33m{new_gain:.2}\x1B[0m");

            let new_c = util::prompt_positive(&format!(
                "Suggested c_bypass is \x1B[1;33m{suggested_c_bypass:.2e}\x1B[0m, select your value: "
            ))?;

            let new_freq = util::rcf(new_r + ze, new_c);
            println!("boosted freq is: \x1B[1;33m{new_freq:.2}\x1B[0m");

            Ok(ACNetwork {
                r_bypass: Some(new_r),
                c_bypass: Some(new_c),
                gain: new_gain,
            })
        } else {
            Ok(ACNetwork {
                r_bypass: None,
                c_bypass: None,
                gain: min_gain,
            })
        }
    }
}

#[derive(Default)]
pub(crate) struct ValveParams {
    pub(crate) vs: f64,
    pub(crate) rp: f64,
    pub(crate) rk: f64,
    pub(crate) ck_bypass: Option<f64>,
    pub(crate) mode: ValveMode,
}

impl ValveParams {
    pub(crate) fn new(args: Vec<String>) -> Result<Self, Error> {
        let mut params = Self::default();

        if let Some(cmd) = args.get(1..) {
            let len = cmd.len();

            let beta = String::from("-dz");
            let hfe = String::from("-koren");
            if cmd.contains(&beta) && cmd.contains(&hfe) {
                return Err(Error::InvalidArgs("Both -dz & -koren are defined at the same time, use only one of them.".to_owned()))
            }
            let mut i = 0;

            while i < len {
                match cmd[i].as_str() {
                    "-valve"           => {},
                    "-vs"              => { params.vs = flag_value(cmd, i)?; i += 1 },
                    "-rp"              => { params.rp = flag_value(cmd, i)?; i += 1 },
                    "-rk"              => { params.rk = flag_value(cmd, i)?; i += 1 },
                    "-ck"              => { params.ck_bypass = Some(flag_value(cmd, i)?); i += 1 },
                    "-dz"              => { params.mode = ValveMode::Dz },
                    "-koren"           => { params.mode = ValveMode::Koren },
                    other              => return Err(Error::InvalidArgs(format!("{other} is invalid argument"))),
                }
                i += 1;
            }
        }

        params.validate()?;

        Ok(params)
    }

    /// Sanity checks on the command line values, run once after parsing.
    pub(crate) fn validate(&self) -> Result<(), Error> {
        for (name, v) in [
            ("-vs", self.vs),
            ("-rp", self.rp),
            ("-rk", self.rk),
        ] {
            if !(v.is_finite() && v > 0.0) {
                return Err(Error::InvalidArgs(format!("{name} must be a positive, finite number")));
            }
        }

        if let Some(ck) = self.ck_bypass {
            if !(ck.is_finite() && ck > 0.0) {
                return Err(Error::InvalidArgs(format!("-ck ({ck:.2e}) must be positive & finite")));
            }
        }

        Ok(())
    }
}

pub(crate) fn get_params() -> Result<Params, Error> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() <= 1 { return Err(Error::NeedsHelp) }

    let bjt = args.contains(&String::from("-bjt"));
    let valve = args.contains(&String::from("-valve"));

    if !(bjt ^ valve) { return Err(Error::InvalidArgs("Pick either -bjt or -valve".to_owned())) }

    if bjt { BjtParams::new(args).map(Params::Bjt) }
    else { ValveParams::new(args).map(Params::Valve) }
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

fn get_mode_and_bias(params: &mut BjtParams, mode_locked: &mut bool, bias_locked: &mut bool, s: &str) -> Result<(), Error> {
    match s {
        "-ce"            => { lock_option(mode_locked, "mode", s)?;  params.mode = BjtMode::CommonEmitter },
        "-cc"            => { lock_option(mode_locked, "mode", s)?;  params.mode = BjtMode::CommonCollector },
        "-vdiv"          => { lock_option(bias_locked, "bias", s)?;  params.bias = BiasConfig::VoltageDivider },
        "-cfb"           => { lock_option(bias_locked, "bias", s)?;  params.bias = BiasConfig::CollectorFeedback },
        "-cfdiv"         => { lock_option(bias_locked, "bias", s)?;  params.bias = BiasConfig::CollectorFeedbackDivider },
        "-split"         => { lock_option(bias_locked, "bias", s)?;  params.bias = BiasConfig::CollectorFeedbackSplit },
        "-fixed"         => { lock_option(bias_locked, "bias", s)?;  params.bias = BiasConfig::FixedBias },
        "-bts"           => { lock_option(bias_locked, "bias", s)?;  params.bias = BiasConfig::Bootstrap },
        "-boost"         => { params.boost = true },
        "-h" | "--help"  => return Err(Error::NeedsHelp),
        _                => return Err(Error::InvalidArgs(s.to_owned()))
    }

    Ok(())
}
