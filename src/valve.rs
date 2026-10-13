use std::io::{Read, Write};
use core::str::FromStr;

use crate::{Error};
use crate::{rpar, rcf};

// const CGK:    f64 = 2.3e-12;
// const CGP:    f64 = 2.4e-12;
// const CPK:    f64 = 0.9e-12;

const MU:     f64 = 103.2;
const MU_P1:  f64 = MU + 1.0;

fn softplus_sigmoid(x: f64) -> (f64, f64) {
    if x > 80.0 {
        (x, 1.0)
    } else if x < -80.0 {
        (1e-12, 1e-12)
    } else {
        let e = x.exp();
        (e.ln_1p(), e / (1.0 + e))
    }
}

/// taken from Dempwolf-Zolzer 12ax7 [model](https://dafx.de/paper-archive/2011/Papers/76_e.pdf)
fn calc_gm(vk: f64, vp: f64) -> f64 {
    const Y:      f64 = 1.26;
    const E:      f64 = 1.314;
    const C:      f64 = 3.4;
    const CG:     f64 = 9.901;

    const MU_INV: f64 = 1.0 / MU;
    const C_INV:  f64 = 1.0 / C;
    const CG_INV: f64 = 1.0 / CG;

    const GYC:    f64 = 2.242e-3 * Y * C;
    const GGECG:  f64 = 6.177e-4 * E * CG;

    let v_gk = -vk;
    let v_pk = vp - vk;

    let vk_eff_arg = C * (v_pk * MU_INV + v_gk);
    let vg_eff_arg = CG * v_gk;

    let (ln1p_exp_k, sig_k) = softplus_sigmoid(vk_eff_arg);
    let (ln1p_exp_g, sig_g) = softplus_sigmoid(vg_eff_arg);

    let vk_eff = (ln1p_exp_k * C_INV).powf(Y);
    let vg_eff = (ln1p_exp_g * CG_INV).powf(E);

    let dik_dvk = GYC * (vk_eff / ln1p_exp_k) * sig_k;
    let dig_dvg = GGECG * (vg_eff / ln1p_exp_g) * sig_g;

    dik_dvk - dig_dvg
}

fn select(set: &str, mut f: impl FnMut(f64)) -> Result<(), Error> {
    let mut stdin = std::io::stdin();
    loop {
        print!("set \x1B[1;33m{set}\x1B[0m (type \x1B[1;32mquit\x1B[0m or \x1B[1;32mq\x1B[0m to abort): ");
        std::io::stdout().flush()?;

        let mut buffer = [0u8; 32];
        let n = stdin.read(&mut buffer)?;
        let s = str::from_utf8(&buffer[..n]).map(str::trim);
        if s.is_ok_and(|q| q == "quit" || q == "q") {
            println!("Aborted");
            return Err(Error::Aborted);
        }
        if let Ok(Ok(new)) = s.map(<f64 as FromStr>::from_str) {
            f(new);
            break;
        }
    }

    Ok(())
}

#[derive(Default)]
pub(crate) struct ValveParams {
    vs: f64,
    vk: f64,
    rp: f64,
    rk: f64,
    ck: f64,
    // cgk: f64,
}

// fn bypassed_gain(rp: f64, rpp: f64) -> f64 {
//     MU * rp / (rp + rpp)
// }

// fn unbypassed_gain(rp: f64, rpp: f64, rk: f64) -> f64 {
//     let denom = rp + rpp + (MU_P1 * rk);
//     MU * rp / denom
// }

/// bypassed   = MU * rp / (rp + rpp)
/// unbypassed = MU * rp / rp + rpp + (MU_P1 * rk)
/// ratio      = bypassed / unbypassed
fn gain_ratio(rp: f64, rpp: f64, rk: f64) -> f64 {
    let r_plate = rp + rpp;
    1.0 + MU_P1 * rk / r_plate
}

impl ValveParams {
    pub(crate) fn get() -> Result<Self, Error> {
        let mut this = ValveParams::default();

        select("v_supply", |vs| this.vs = vs)?;
        select("v_cathode", |vk| this.vk = vk)?;
        select("Plate resistor", |rp| this.rp = rp)?;
        select("Cathode resistor", |rk| this.rk = rk)?;
        select("Cathode bypass capacitor", |ck| this.ck = ck)?;
        // select("cgk", |cgk| this.cgk = cgk)?;

        Ok(this)
    }

    pub(crate) fn ac_analysis(&self, zo: f64, re: f64, ree: f64) -> Result<ACNetwork, Error> {
        let ik = self.vk / self.rk;
        let vrp = self.rp * ik;
        let vp = self.vs - vrp;
        let gm = calc_gm(self.vk, vp);
        let rpp = MU / gm;

        let rkk = (self.rp + rpp) / MU_P1;
        let rk_eq = rpar!(self.rk, rkk);
        let boosted_freq = rcf(rk_eq, self.ck);

        let ratio = gain_ratio(self.rp, rpp, self.rk);
        let bjt_gain = zo / (re + ree);
        let expected_gain = bjt_gain * ratio;

        // re || r_bypass = zo / expected_gain - ree
        // re * rbp / (re + rbp) = zo/xg - ree -> re * rbp / (re + rbp) = a
        // re * rbp = a * re + a * rbp
        // re * rbp - a * rbp = a * re
        // rbp (re - a) = a * re
        // rbp = a * re / (re - a)
        // rbp = re * (zo/xg - ree) / (re - (zo/xg - ree))
        let a = zo / expected_gain - ree;
        let mut r_bypass = re * a / (re - a);
        crate::bjt::select("Suggested r_bypass", r_bypass, |new| Ok(r_bypass = new))?;
        let ze = rpar!(re, ree);
        let c_bypass = rcf(r_bypass + ze, boosted_freq);

        // let gk_roll_off = rcf(rk_eq, CGK + self.cgk);
        // let mut cbe = rcf(ze, gk_roll_off);
        // crate::bjt::select("Suggested base-emitter capacitor", cbe, |new| Ok(cbe = new))?;

        // let rp_eq = rpar!(self.rp, rpp);
        // let gp_roll_off = rcf(rp_eq, CGP);
        // let mut cbc = rcf(zo, gp_roll_off);
        // crate::bjt::select("Suggested collector capacitor", cbc, |new| Ok(cbc = new))?;

        Ok(ACNetwork {
            r_bypass,
            c_bypass,
            // cbe,
            // cbc,
        })
    }
}

pub(crate) struct ACNetwork {
    pub(crate) r_bypass: f64,
    pub(crate) c_bypass: f64,
    // pub(crate) cbe: f64,
    // pub(crate) cbc: f64,
}

impl core::fmt::Debug for ACNetwork {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "r_bypass         : {:.2e}\n", self.r_bypass)?;
        write!(f, "c_bypass         : {:.2e}", self.c_bypass)
    }
}
