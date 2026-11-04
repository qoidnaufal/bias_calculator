use std::io::{Read, Write};

use crate::{Error};
use crate::{rpar, rcf};

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
fn calc_g(vk: f64, vp: f64) -> f64 {
    const Y:      f64 = 1.26;
    // const E:      f64 = 1.314;
    const C:      f64 = 3.4;
    // const CG:     f64 = 9.901;

    const MU_INV: f64 = 1.0 / MU;
    const C_INV:  f64 = 1.0 / C;
    // const CG_INV: f64 = 1.0 / CG;

    const GYC:    f64 = 2.242e-3 * Y * C;
    // const GGECG:  f64 = 6.177e-4 * E * CG;

    let v_gk = -vk;
    let v_pk = vp - vk;

    let vk_eff_arg = C * (v_pk * MU_INV + v_gk);
    // let vg_eff_arg = CG * v_gk;

    let (ln1p_exp_k, sig_k) = softplus_sigmoid(vk_eff_arg);
    // let (ln1p_exp_g, sig_g) = softplus_sigmoid(vg_eff_arg);

    let vk_eff = (ln1p_exp_k * C_INV).powf(Y);
    // let vg_eff = (ln1p_exp_g * CG_INV).powf(E);

    let dik_dvk = GYC * (vk_eff / ln1p_exp_k) * sig_k;
    // let dig_dvg = GGECG * (vg_eff / ln1p_exp_g) * sig_g;

    dik_dvk

    // dik_dvk - dig_dvg
}

/// Norman Koren's model
fn get_ip(vpk: f64, vgk: f64) -> f64 {
    const MU_INV: f64 = 1e-2;
    const EX: f64 = 1.4;
    const KG_INV: f64 = 1.0 / 1060.0;
    const KP: f64 = 6e2;
    const KP_INV: f64 = 1.0 / 6e2;
    const KVB: f64 = 3e2;
    // const RGI: f64 = 2e3;

    if vpk <= 0.0 { return 0.0 }
    let sqrt = f64::sqrt(f64::mul_add(vpk, vpk, KVB));
    let exp = f64::exp(KP * (MU_INV + vgk / sqrt));
    let e1 = vpk * KP_INV * f64::ln_1p(exp);
    (f64::powf(e1, EX) * KG_INV) * (1.0 + f64::signum(e1))
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
        if let Ok(Ok(new)) = s.map(crate::parse) {
            f(new);
            break;
        }
    }

    Ok(())
}

pub(crate) struct ValveParams {
    vs: f64,
    vk: f64,
    rp: f64,
    rk: f64,
    ck: f64,
    mode: Mode,
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) enum Mode {
    Dz,
    Koren
}

/// bypassed   = MU * rp / (rp + rpp)
/// unbypassed = MU * rp / rp + rpp + (MU_P1 * rk)
/// ratio      = bypassed / unbypassed
/// (rp + rpp + (MU_P1 * rk)) / (rp + rpp)
/// = a + (MU_P1 * rk) / a
/// = 1 + (MU_P1 * rk / a)
fn gain_ratio(mu_p1: f64, rp: f64, rpp: f64, rk: f64) -> f64 {
    let a = rp + rpp;
    1.0 + (mu_p1 * rk / a)
}

impl ValveParams {
    pub(crate) fn get(mode: Mode) -> Result<Self, Error> {
        let mut this = ValveParams {
            vs: 0.0,
            vk: 0.0,
            rp: 0.0,
            rk: 0.0,
            ck: 0.0,
            mode,
        };

        select("v_supply", |vs| this.vs = vs)?;
        if mode == Mode::Dz { select("v_cathode", |vk| this.vk = vk)? }
        select("Plate resistor", |rp| this.rp = rp)?;
        select("Cathode resistor", |rk| this.rk = rk)?;
        select("Cathode bypass capacitor", |ck| this.ck = ck)?;

        Ok(this)
    }

    pub(crate) fn ac_analysis(&self, zo: f64, re: f64, ree: f64) -> Result<ACNetwork, Error> {
        let (boosted_freq, ratio) = match self.mode {
            Mode::Dz    => self.dempwolf_zolzer(),
            Mode::Koren => self.koren(),
        };

        let bjt_gain = zo / (re + ree);
        let expected_gain = bjt_gain * ratio;

        // re || r_bypass = zo / expected_gain - ree
        // re * rbp / (re + rbp) = zo/xg - ree <=> [zo/xg - ree => a]
        // re * rbp = a * re + a * rbp
        // re * rbp - a * rbp = a * re
        // rbp (re - a) = a * re
        // rbp = a * re / (re - a)
        // rbp = re * (zo/xg - ree) / (re - (zo/xg - ree))
        let a = zo / expected_gain - ree;
        let mut r_bypass = crate::par_extract(a, re);
        crate::bjt::select("Suggested r_bypass", r_bypass, |new| Ok(r_bypass = new))?;
        let ze = rpar!(re, ree);
        let c_bypass = rcf(r_bypass + ze, boosted_freq);

        let ac_re = rpar!(r_bypass, re);
        let gain = zo / (ac_re + ree);

        Ok(ACNetwork {
            r_bypass,
            c_bypass,
            gain,
        })
    }

    fn dempwolf_zolzer(&self) -> (f64, f64) {
        let ik = self.vk / self.rk;
        let vrp = self.rp * ik;
        let vp = self.vs - vrp;
        let g = calc_g(self.vk, vp);
        let rpp = MU / g;

        let rkk = (self.rp + rpp) / MU_P1;
        let rk_eq = rpar!(self.rk, rkk);
        let boosted_freq = rcf(rk_eq, self.ck);
        let ratio = gain_ratio(MU_P1, self.rp, rpp, self.rk);

        println!(" > \x1B[1;33mvp\x1B[0m : {vp:.2} | \x1B[1;33mgain ratio\x1B[0m : {ratio:.2} | \x1B[1;33mfreq\x1B[0m : {boosted_freq:.2}");

        (boosted_freq, ratio)
    }

    fn koren(&self) -> (f64, f64) {
        const H: f64 = 1e-3;
        const H2: f64 = 2e-3;

        let rt = self.rp + self.rk;
        let mut lo = 0f64;
        let mut hi = self.vs / rt;

        for _ in 0..100 {
            let i = (lo + hi) / 2.0;
            let ip = get_ip(self.vs - i * rt, -i * self.rk);
            if ip > i { lo = i } else { hi = i }
        }

        let i = (lo + hi) / 2.0;
        let vpk = self.vs - i * rt;
        let vk = i * self.rk;
        let vgk = -vk;

        let gm = (get_ip(vpk, vgk + H) - get_ip(vpk, vgk - H)) / H2; 
        let gp = (get_ip(vpk + 0.1, vgk) - get_ip(vpk - 0.1, vgk)) / 0.2;

        let rpp = 1.0 / gp;
        let mu_p1 = gm * rpp + 1.0;
        let rkk = (self.rp + rpp) / mu_p1;
        let rk_eq = rpar!(self.rk, rkk);
        let boosted_freq = rcf(rk_eq, self.ck);
        let ratio = gain_ratio(mu_p1, self.rp, rpp, self.rk);

        let vp = self.vs - i * self.rp;
        println!(" > \x1B[1;33mvp\x1B[0m : {vp:.2} | \x1B[1;33mvk\x1B[0m : {vk:.2} | \x1B[1;33mgain ratio\x1B[0m : {ratio:.2} | \x1B[1;33mfreq\x1B[0m : {boosted_freq:.2}");

        (boosted_freq, ratio)
    }
}

pub(crate) struct ACNetwork {
    pub(crate) r_bypass: f64,
    pub(crate) c_bypass: f64,
    pub(crate) gain: f64,
}

impl core::fmt::Debug for ACNetwork {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "r_bypass         : {:.2e}\n", self.r_bypass)?;
        write!(f, "c_bypass         : {:.2e}\n", self.c_bypass)?;
        write!(f, "ac gain          : {:.2}", self.gain)?;

        Ok(())
    }
}
