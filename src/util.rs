use std::io::{Write, Read};

use crate::parser::parse;

#[macro_export]
macro_rules! rpar {
    ($($r:expr),* $(,)?) => {{
        let mut g = 0.0;
        $(g += 1.0/$r;)*
        1.0/g
    }};
}

pub(crate) fn par_extract(par: f64, known: f64) -> f64 {
    known * par / (known - par)
}

pub(crate) fn rcf(rcf1: f64, rcf2: f64) -> f64 {
    const PI2: f64 = 2.0 * core::f64::consts::PI;
    1.0 / (PI2 * rcf1 * rcf2)
}

pub(crate) fn read_buffer<'a>(stdin: &'a mut std::io::Stdin, buffer: &'a mut [u8; 32]) -> Result<&'a str, Error> {
    let n = stdin.read(buffer)?;
    str::from_utf8(&buffer[..n])
        .map(str::trim)
        .map_err(|err| Error::Io(err.to_string()))
}

#[derive(Debug)]
pub(crate) enum Error {
    NumParse(String),
    InvalidArgs(String),
    NeedsHelp,
    DiscouragedMode,
    Aborted,
    Io(String),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::NumParse(msg)           => write!(f, "Unable to parse int/float: {msg}"),
            Error::InvalidArgs(msg)        => write!(f, "Invalid args: {msg}"),
            Error::NeedsHelp               => write!(f, "Needs help"),
            Error::DiscouragedMode         => write!(f, "This mode & bias is discouraged"),
            Error::Aborted                 => write!(f, "Aborted"),
            Error::Io(io)                  => write!(f, "Io: {io}"),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

impl std::error::Error for Error {}

pub(crate) struct SelectError;

/// Asks for a strictly positive, finite number. `q` / `quit` aborts.
pub(crate) fn prompt_positive(msg: &str) -> Result<f64, Error> {
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
        if let Ok(Ok(v)) = s.map(parse) {
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
        if let Ok(Ok(new)) = s.map(parse) {
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
        if let Ok(Ok(new)) = s.map(parse) {
            if new.is_finite() && new > 0.0 && new <= max {
                *target = new;
                break
            }
            println!("The value must satisfy 0 < {what} <= {max:.2e}.");
        }
    }

    Ok(())
}
