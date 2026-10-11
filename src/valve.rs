use std::io::{Read, Write};
use core::str::FromStr;

use crate::{Error};
use crate::{rpar, rcf};

fn softplus_sigmoid(x: f64, limit: f64) -> (f64, f64) {
    if x > limit {
        (x, 1.0)
    } else if x < -limit {
        (1e-12, 1e-12)
    } else {
        let e = x.exp();
        (e.ln_1p(), e / (1.0 + e))
    }
}

fn v_gain(rp: f64, rpp: f64) -> f64 {
    MU * (rp / (rp + rpp))
}

const MU: f64 = 103.2;
const Y: f64 = 1.26;
const C: f64 = 3.4;
const E: f64 = 1.314;
const CG: f64 = 9.901;

const MU_INV: f64 = 1.0 / MU;
const C_INV: f64 = 1.0 / C;
const CG_INV: f64 = 1.0 / CG;
const MU_P1: f64 = MU + 1.0;

const GYC: f64 = 2.242e-3 * Y * C;
const GGECG: f64 = 6.177e-4 * E * CG;

fn calc_gm(vk: f64, vp: f64) -> f64 {
    let v_gk = -vk;
    let v_pk = vp - vk;

    let vk_eff_arg = C * (v_pk * MU_INV + v_gk);
    let vg_eff_arg = CG * v_gk;

    let (ln1p_exp_k, sig_k) = softplus_sigmoid(vk_eff_arg, 80.0);
    let (ln1p_exp_g, sig_g) = softplus_sigmoid(vg_eff_arg, 80.0);

    let vk_eff = (ln1p_exp_k * C_INV).powf(Y);
    let vg_eff = (ln1p_exp_g * CG_INV).powf(E);

    let dik_dvk = GYC * (vk_eff / ln1p_exp_k) * sig_k;
    let dig_dvg = GGECG * (vg_eff / ln1p_exp_g) * sig_g;

    dik_dvk - dig_dvg
}

pub(crate) struct ValveParams {
    rp: f64,
    rk: f64,
    ck: f64,
    vk: f64,
    vs: f64,
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

impl ValveParams {
    pub(crate) fn get() -> Result<Self, Error> {
        let mut this = Self {
            rp: 0.0,
            rk: 0.0,
            ck: 0.0,
            vk: 0.0,
            vs: 0.0,
        };
        select("rp", |rp| this.rp = rp)?;
        select("rk", |rk| this.rk = rk)?;
        select("ck", |ck| this.ck = ck)?;
        select("vk", |vk| this.vk = vk)?;
        select("vs", |vs| this.vs = vs)?;

        Ok(this)
    }

    // pub(crate) fn analyze(&self) -> Result<(), Error> {
    //     let ik = self.vk / self.rk;
    //     let vrp = self.rp * ik;
    //     let vp = self.vs - vrp;
    //     let gm = calc_gm(self.vk, vp);
    //     let rpp = MU / gm;

    //     let rkk = (self.rp + rpp) / MU_P1;
    //     let req = rpar!(self.rk, rkk);
    //     let boosted = rcf(req, self.ck);

    //     let gain = v_gain(self.rp, rpp);
    //     let db = db(gain);

    //     print!("rkk              : {rkk:.2e}\n");
    //     print!("req              : {req:.2e}\n");
    //     print!("boosted freq     : {boosted:.2e}hz +{db:2} db\n");
    //     std::io::stdout().flush()?;

    //     Ok(())
    // }

    pub(crate) fn analyze_bjt(&self, rc: f64, re: f64, ree: f64) -> Result<RCBypass, Error> {
        let ze = rpar!(re, ree);

        let ik = self.vk / self.rk;
        let vrp = self.rp * ik;
        let vp = self.vs - vrp;
        let gm = calc_gm(self.vk, vp);
        let rpp = MU / gm;

        let rkk = (self.rp + rpp) / MU_P1;
        let req = rpar!(self.rk, rkk);
        let boosted = rcf(req, self.ck);
        let gain = v_gain(self.rp, rpp);

        let mut r_bypass = rc / gain - ze;
        crate::bjt::select("Suggested r_bypass", r_bypass, |new| Ok(r_bypass = new))?;
        let c_bypass = rcf(r_bypass, boosted);

        // print!("r_bypass         : {r_bypass:.2e}\n");
        // print!("c_bypass         : {c_bypass:.2e}\n");
        // std::io::stdout().flush()?;

        Ok(RCBypass { r_bypass, c_bypass })
    }
}

pub(crate) struct RCBypass {
    pub(crate) r_bypass: f64,
    pub(crate) c_bypass: f64,
}
