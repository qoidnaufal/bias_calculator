use std::io::Write;

use crate::Error;
use crate::{rpar, rcf, read_buffer};

const MU:    f64 = 103.2;
const MU_P1: f64 = MU + 1.0;

// Dempwolf-Zolzer 12ax7 parameters
const DZ_G: f64 = 2.242e-3;
const DZ_Y: f64 = 1.26;
const DZ_C: f64 = 3.4;

/// returns (ln(1 + e^x), sigmoid(x)) without overflow
fn softplus_sigmoid(x: f64) -> (f64, f64) {
    if x > 80.0 {
        (x, 1.0)
    } else if x < -80.0 {
        // ln(1 + e^x) ~ e^x and sigmoid(x) ~ e^x; clamp so a later division never sees 0
        let e = x.exp().max(f64::MIN_POSITIVE);
        (e, e)
    } else {
        let e = x.exp();
        (e.ln_1p(), e / (1.0 + e))
    }
}

/// Dempwolf-Zolzer 12ax7 [model](https://dafx.de/paper-archive/2011/Papers/76_e.pdf)
/// Ik = G * (ln(1 + exp(C * (vpk / MU + vgk))) / C)^Y
fn dz_ik(vpk: f64, vgk: f64) -> f64 {
    let (sp, _) = softplus_sigmoid(DZ_C * (vpk / MU + vgk));
    DZ_G * (sp / DZ_C).powf(DZ_Y)
}

/// transconductance dIk/dVgk of the Dempwolf-Zolzer model (plate resistance is MU / gm)
fn dz_gm(vpk: f64, vgk: f64) -> f64 {
    let (sp, sig) = softplus_sigmoid(DZ_C * (vpk / MU + vgk));
    DZ_G * DZ_Y * (sp / DZ_C).powf(DZ_Y - 1.0) * sig
}

/// Norman Koren's model
fn get_ip(vpk: f64, vgk: f64) -> f64 {
    const MU_INV: f64 = 1e-2;
    const EX: f64 = 1.4;
    const KG_INV: f64 = 1.0 / 1060.0;
    const KP: f64 = 6e2;
    const KP_INV: f64 = 1.0 / 6e2;
    const KVB: f64 = 3e2;

    if vpk <= 0.0 { return 0.0 }
    let sqrt = f64::sqrt(f64::mul_add(vpk, vpk, KVB));
    let (sp, _) = softplus_sigmoid(KP * (MU_INV + vgk / sqrt));
    let e1 = vpk * KP_INV * sp; // > 0 here, so sgn(e1) = 1 in Koren's formula
    2.0 * f64::powf(e1, EX) * KG_INV
}

/// DC operating point of a common-cathode stage with the grid at 0 V:
/// finds the cathode current i such that ik(vs - i * (rp + rk), -i * rk) = i.
/// `ik(vpk, vgk)` decreases with i, so `ik(..) - i` is monotonic and bisection is valid.
fn solve_dc(vs: f64, rp: f64, rk: f64, ik: impl Fn(f64, f64) -> f64) -> f64 {
    let rt = rp + rk;
    let mut lo = 0f64;
    let mut hi = vs / rt;

    for _ in 0..100 {
        let i = (lo + hi) / 2.0;
        if ik(vs - i * rt, -i * rk) > i { lo = i } else { hi = i }
    }

    (lo + hi) / 2.0
}

/// asks for a strictly positive, finite value. `optional` values may be skipped with `no` or `n`
fn select(set: &str, optional: bool, mut f: impl FnMut(f64)) -> Result<(), Error> {
    let mut stdin = std::io::stdin();
    loop {
        print!("set \x1B[1;33m{set}\x1B[0m (type \x1B[1;32mquit\x1B[0m or \x1B[1;32mq\x1B[0m to abort): ");
        std::io::stdout().flush()?;

        let mut buffer = [0u8; 32];
        let s = read_buffer(&mut stdin, &mut buffer);

        if s.as_ref().is_ok_and(|q| *q == "quit" || *q == "q") {
            println!("Aborted");
            return Err(Error::Aborted);
        }

        if optional && s.as_ref().is_ok_and(|q| *q == "no" || *q == "n") {
            break;
        }

        if let Ok(Ok(new)) = s.map(crate::parse) {
            if new.is_finite() && new > 0.0 {
                f(new);
                break;
            }
        }
    }

    Ok(())
}

/// `c_bypass` is already fixed (it was chosen for the fully bypassed position), so the pole of the
/// partially bypassed position is determined by `r_bypass`: f = 1 / (2 pi (r_bypass + ze) c_bypass)
pub(crate) fn select_bypass(
    r: &mut f64,
    c: &mut f64,
    ree: f64,
    re: f64,
    ze: f64,
    zo: f64,
    freq: f64,
) -> Result<(), Error> {
    let mut stdin = std::io::stdin();
    loop {
        print!(
            "Suggested r_bypass: \x1B[1;33m{:.2e}\x1B[0m (capacitor at {:.2e}), select your value (type \x1B[1;32mquit\x1B[0m or \x1B[1;32mq\x1B[0m to abort): ",
            *r, *c
        );
        std::io::stdout().flush()?;

        let mut buffer = [0u8; 32];
        let s = read_buffer(&mut stdin, &mut buffer);

        if s.as_ref().is_ok_and(|q| *q == "quit" || *q == "q") {
            println!("Aborted");
            return Err(Error::Aborted);
        }

        if let Ok(Ok(new_r)) = s.map(crate::parse) {
            if !(new_r.is_finite() && new_r > 0.0) { continue }

            let new_c = rcf(new_r + ze, freq);
            let ac_re = rpar!(new_r, re);
            let gain = zo / (ac_re + ree);

            print!("ac_gain will be: {gain:.2}, capacitor will be: {new_c:.2e}, continue? ");
            std::io::stdout().flush()?;

            let mut buffer = [0u8; 32];
            let s = read_buffer(&mut stdin, &mut buffer);

            if s.is_ok_and(|y| y == "yes" || y == "y") {
                *r = new_r;
                *c = new_c;
                break;
            }
        }
    }

    Ok(())
}

pub(crate) struct ValveParams {
    vs: f64,
    rp: f64,
    rk: f64,
    ck_bypass: f64,
    // /// in some tube design, a bypass capacitor will be connected in series with a resistor
    // /// and then a switch is added to toggle between bypassing the resistor or not
    // rk_bypass: Option<f64>,
    mode: Mode,
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) enum Mode {
    Dz,
    Koren
}

/// With a cathode impedance zk, the stage gain is
/// G(zk)     = MU * rp / (rp + rpp + MU_P1 * zk)
/// a         = rp + rpp.
///
/// Ratios are taken against the unbypassed gain (zk = rk, capacitor open):
/// ratio(zk) = G(zk) / G(rk)
/// = (a + MU_P1 * rk) / (a + MU_P1 * zk)
/// = 1 + MU_P1 * rk / a
fn gain_ratio(mu_p1: f64, rp: f64, rpp: f64, rk1: f64) -> f64 {
    let a = rp + rpp;
    let unbypassed = a + mu_p1 * rk1;
    unbypassed / a
}

impl ValveParams {
    pub(crate) fn get(mode: Mode) -> Result<Self, Error> {
        let mut this = ValveParams {
            vs: 0.0,
            rp: 0.0,
            rk: 0.0,
            ck_bypass: 0.0,
            // rk_bypass: None,
            mode,
        };

        // the cathode voltage is no longer asked for: it is solved from vs, rp and rk
        // so that the DC operating point is consistent with the tube model
        select("v_supply", false, |vs| this.vs = vs)?;
        select("Plate resistor", false, |rp| this.rp = rp)?;
        select("Cathode resistor", false, |rk| this.rk = rk)?;
        // select("Cathode bypass resistor", true, |rk_bypass| this.rk_bypass = Some(rk_bypass))?;
        select("Cathode bypass capacitor", false, |ck_bypass| this.ck_bypass = ck_bypass)?;

        Ok(this)
    }

    /// Fully bypassed: ree stays in series, re is shorted by C. Gain = zo / ree.
    /// Unbypassed (C open): gain = zo / (re + ree).
    /// With C in series with rb across re:
    /// Z       = re || (rb + 1/sC)
    /// gain(s) = zo / (ree + Z), pole at 1 / (2 pi (rb + ze) C), ze = re || ree
    /// (rb = 0 gives the fully bypassed case, so C = 1 / (2 pi ze f1)).
    ///
    /// Mid-band, with the capacitor shorted, the partial gain is zo / ((re || rb) + ree), so
    /// re || rb = zo / expected_gain - ree = a, which only has a solution for 0 < a < re:
    /// re * rb / (re + rb) = a  =>  rb = a * re / (re - a)
    pub(crate) fn ac_analysis(&self, zo: f64, re: f64, ree: f64) -> Result<ACNetwork, Error> {
        let (boosted_freq, ratio) = match self.mode {
            Mode::Dz    => self.dempwolf_zolzer(),
            Mode::Koren => self.koren(),
        };

        let bjt_gain = zo / (re + ree);
        println!(" > \x1B[1;33mtube gain ratio\x1B[0m: : {ratio:.2}");

        let gain = crate::bjt::select_positive("gain", bjt_gain)?;
        let a = zo / gain - ree;
        let ze = rpar!(re, ree);
        let mut r_bypass = crate::par_extract(a, re);
        let mut c_bypass = rcf(ze + r_bypass, boosted_freq);

        select_bypass(&mut r_bypass, &mut c_bypass, ree, re, ze, zo, boosted_freq)?;

        let ac_re = rpar!(r_bypass, re);
        let gain = zo / (ac_re + ree);

        // if let Some(freq2) = partial {
        //     let partial_z = rcf(c_bypass, freq2);
        //     let partial_r_bypass = crate::bjt::select_positive("partial_r_bypass", partial_z - (r_bypass + ze))?;
        //     let shifted_freq = rcf(partial_r_bypass, c_bypass);

        //     println!(" > \x1B[1;33mpartial_r_bypass\x1B[0m : {partial_r_bypass:.2} | \x1B[1;33mshifted freq\x1B[0m : {shifted_freq:.2}", );
        // };

        Ok(ACNetwork {
            r_bypass: Some(r_bypass),
            c_bypass: Some(c_bypass),
            gain,
        })
    }

    /// shared small signal part: given mu + 1 and the plate resistance at the operating point
    /// returns (pole frequency, gain ratio, Option<(pole frequency, gain ratio)> of the partial case)
    ///
    /// cathode impedance seen by the capacitor: rk || rkk, with rkk = (rp + rpp) / (mu + 1)
    /// fully bypassed pole : 1 / (2 pi (rk || rkk) Ck)
    /// with rs in series   : 1 / (2 pi (rs + rk || rkk) Ck)
    fn small_signal(&self, mu_p1: f64, rpp: f64) -> (f64, f64) {
        let rkk = (self.rp + rpp) / mu_p1;
        let rk_eq1 = rpar!(self.rk, rkk);

        let boosted_freq = rcf(rk_eq1, self.ck_bypass);
        // let shifted_freq = self.rk_bypass.map(|rs| rcf(rk_eq1 + rs, self.ck_bypass));
        let ratio = gain_ratio(mu_p1, self.rp, rpp, self.rk);

        println!(" > \x1B[1;33mgain ratio\x1B[0m : {ratio:.2}", );
        // if let Some(freq) = shifted_freq {
        //     println!(" > \x1B[1;33mboosted freq\x1B[0m : {freq:.2} - {boosted_freq:.2}", );
        // } else {
        //     println!(" > \x1B[1;33mboosted freq\x1B[0m : {boosted_freq:.2}", );
        // }
        println!(" > \x1B[1;33mboosted freq\x1B[0m : {boosted_freq:.2}", );

        (boosted_freq, ratio)
    }

    /// (freq, gain ratio, partial freq)
    fn dempwolf_zolzer(&self) -> (f64, f64) {
        let i = solve_dc(self.vs, self.rp, self.rk, dz_ik);
        let vk = i * self.rk;
        let vp = self.vs - i * self.rp;
        let vpk = vp - vk;

        let gm = dz_gm(vpk, -vk);
        let rpp = MU / gm;

        println!(" > \x1B[1;33mvp\x1B[0m : {vp:.2} | \x1B[1;33mvk\x1B[0m : {vk:.2}");
        self.small_signal(MU_P1, rpp)
    }

    /// (freq, gain ratio, partial freq, partial gain ratio)
    fn koren(&self) -> (f64, f64) {
        const H: f64 = 1e-3;
        const H2: f64 = 2e-3;

        let i = solve_dc(self.vs, self.rp, self.rk, get_ip);
        let vpk = self.vs - i * (self.rp + self.rk);
        let vk = i * self.rk;
        let vgk = -vk;

        let hp = f64::min(0.1, 0.5 * vpk); // keep vpk - hp > 0, get_ip is clamped at vpk <= 0
        let gm = (get_ip(vpk, vgk + H) - get_ip(vpk, vgk - H)) / H2;
        let gp = (get_ip(vpk + hp, vgk) - get_ip(vpk - hp, vgk)) / (2.0 * hp);

        let rpp = 1.0 / gp;
        let mu_p1 = gm * rpp + 1.0;

        let vp = self.vs - i * self.rp;
        println!(" > \x1B[1;33mvp\x1B[0m : {vp:.2} | \x1B[1;33mvk\x1B[0m : {vk:.2}");
        self.small_signal(mu_p1, rpp)
    }
}

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
