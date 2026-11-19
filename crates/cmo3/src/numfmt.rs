//! Number spelling for element text: the editor writes primitives with `toString()`, so a Rust float
//! must lay out like `Float.toString`/`Double.toString` (JDK 19+, shortest round-trip digits) for
//! a re-emitted main.xml to stay byte-identical.
//!
//! Layout rules: 1e-3 <= |x| < 1e7 prints as plain decimal with at least one fraction digit
//! (`100.0`, `0.001`); anything else prints as computerized scientific notation `d.dddE±n`
//! (`1.0E7`, `1.5E-5`). Zero is `0.0` or `-0.0`; NaN and infinities print as `NaN`,
//! `Infinity`, `-Infinity`.
//!
//! Digit selection: Java (JDK 19+) picks the shortest decimal that round-trips, but never fewer
//! than two significant digits, and among candidates of that length the one closest to the exact
//! value, breaking an exact tie to the even digit. An editor bundling a pre-19 JDK would break
//! ties upward instead; no 5.4-era sample is at hand to settle which it does.
//! Rust's `{:e}` gives the shortest; when that is a single digit the two-digit form is taken from
//! `{:.1e}` (correctly rounded from the exact value), which is what makes `Float.MIN_VALUE`
//! print as `1.4E-45` rather than `1.0E-45`.

use crate::{Result, bail};

/// `java.lang.Float.toString(f)`.
pub fn float_to_string(v: f32) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    layout(&shortest_f32(v, true), v.is_sign_negative())
}

/// `java.lang.Double.toString(d)`.
pub fn double_to_string(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    layout(&shortest_f64(v, true), v.is_sign_negative())
}

/// Plain-decimal spelling: never scientific, `.0` on integral values (`0.00024414063`,
/// `1781.0`). What the indented 5.2 writer emits. Its tie rule differs from a modern JDK: an
/// exact decimal tie rounds up (`2^-12` -> `0.00024414063`, observed), which is what Rust's
/// shortest repr already does, so no tie correction is applied here.
pub fn float_to_plain_string(v: f32) -> String {
    if v.is_nan() || v.is_infinite() {
        return float_to_string(v);
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    layout_plain(&shortest_f32(v, false), v.is_sign_negative())
}

/// Plain-decimal `Double` spelling, see [`float_to_plain_string`].
pub fn double_to_plain_string(v: f64) -> String {
    if v.is_nan() || v.is_infinite() {
        return double_to_string(v);
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    layout_plain(&shortest_f64(v, false), v.is_sign_negative())
}

/// The shortest digit string for an f32 as Rust-style scientific text, with the two-digit
/// minimum and, when `even_ties` is set, JDK 19+ tie breaking.
fn shortest_f32(v: f32, even_ties: bool) -> String {
    let shortest = min_two_digits(format!("{:e}", v), || format!("{:.1e}", v));
    if !even_ties {
        return shortest;
    }
    let (m, e) = decompose_f32(v);
    even_tie(shortest, m, e, |s| s.parse::<f32>().ok() == Some(v.abs()))
}

/// f64 twin of [`shortest_f32`].
fn shortest_f64(v: f64, even_ties: bool) -> String {
    let shortest = min_two_digits(format!("{:e}", v), || format!("{:.1e}", v));
    if !even_ties {
        return shortest;
    }
    let (m, e) = decompose_f64(v);
    even_tie(shortest, m, e, |s| s.parse::<f64>().ok() == Some(v.abs()))
}

/// `|v| = m * 2^e` exactly.
fn decompose_f32(v: f32) -> (u64, i32) {
    let bits = v.abs().to_bits();
    let exp = ((bits >> 23) & 0xFF) as i32;
    let frac = (bits & 0x7F_FFFF) as u64;
    if exp == 0 { (frac, -149) } else { (frac | (1 << 23), exp - 150) }
}

fn decompose_f64(v: f64) -> (u64, i32) {
    let bits = v.abs().to_bits();
    let exp = ((bits >> 52) & 0x7FF) as i32;
    let frac = bits & 0xF_FFFF_FFFF_FFFF;
    if exp == 0 { (frac, -1074) } else { (frac | (1 << 52), exp - 1075) }
}

/// Breaks an exact decimal tie to the even digit, as the editor's `toString` does.
///
/// Rust's `{:e}` is the shortest round-trip repr but rounds a tie away from zero
/// (2133.03125f32 -> `2.1330313e3`); Java picks the candidate closest to the exact value and, on
/// an exact tie, the even last digit (`2133.0312`). A tie means the exact decimal expansion of
/// the value has exactly one more significant digit than the shortest repr and that digit is 5.
fn even_tie(shortest: String, m: u64, e: i32, round_trips: impl Fn(&str) -> bool) -> String {
    let (digits, exp) = split_sci(&shortest);
    let (exact, exact_exp) = exact_decimal(m, e);
    if exact.len() != digits.len() + 1 || !exact.ends_with('5') {
        return shortest;
    }
    let prefix = &exact[..digits.len()];
    let last = prefix.as_bytes()[prefix.len() - 1] - b'0';
    if !last.is_multiple_of(2) {
        return shortest; // the rounded-up candidate is the even one
    }
    let candidate = {
        let trimmed = prefix.trim_end_matches('0');
        let trimmed = if trimmed.is_empty() { "0" } else { trimmed };
        let (head, tail) = trimmed.split_at(1);
        if tail.is_empty() {
            format!("{head}e{exact_exp}")
        } else {
            format!("{head}.{tail}e{exact_exp}")
        }
    };
    // Same length as Rust's pick (so still shortest) and still round-trips, else keep Rust's.
    if exact_exp == exp || digits.len() > 1 {
        if round_trips(&candidate) { candidate } else { shortest }
    } else {
        shortest
    }
}

/// The exact decimal significant digits of `m * 2^e` (no trailing zeros) and the scientific
/// exponent of the first digit.
fn exact_decimal(m: u64, e: i32) -> (String, i32) {
    if m == 0 {
        return ("0".into(), 0);
    }
    // m * 2^e = (m * 2^max(e,0) * 5^max(-e,0)) / 10^max(-e,0)
    let mut big = BigDec::from_u64(m);
    for _ in 0..e.max(0) {
        big.mul_small(2);
    }
    for _ in 0..(-e).max(0) {
        big.mul_small(5);
    }
    let text = big.to_string();
    let scale = (-e).max(0);
    let sci_exp = text.len() as i32 - 1 - scale;
    let digits = text.trim_end_matches('0');
    (if digits.is_empty() { "0".into() } else { digits.to_string() }, sci_exp)
}

/// A tiny unsigned bignum (base 1e9, little-endian limbs) for exact decimal expansion.
struct BigDec(Vec<u32>);

impl BigDec {
    const BASE: u64 = 1_000_000_000;

    fn from_u64(mut v: u64) -> Self {
        let mut limbs = Vec::new();
        while v > 0 {
            limbs.push((v % Self::BASE) as u32);
            v /= Self::BASE;
        }
        BigDec(limbs)
    }

    fn mul_small(&mut self, k: u64) {
        let mut carry = 0u64;
        for limb in &mut self.0 {
            let t = (*limb as u64) * k + carry;
            *limb = (t % Self::BASE) as u32;
            carry = t / Self::BASE;
        }
        while carry > 0 {
            self.0.push((carry % Self::BASE) as u32);
            carry /= Self::BASE;
        }
    }
}

impl std::fmt::Display for BigDec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut it = self.0.iter().rev();
        match it.next() {
            Some(top) => write!(f, "{top}")?,
            None => return write!(f, "0"),
        }
        for limb in it {
            write!(f, "{limb:09}")?;
        }
        Ok(())
    }
}

fn split_sci(sci: &str) -> (String, i32) {
    let body = sci.strip_prefix('-').unwrap_or(sci);
    let (mantissa, exp) = body.split_once('e').expect("{:e} always has an exponent");
    let exp: i32 = exp.parse().expect("exponent is an integer");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let digits = digits.trim_end_matches('0');
    (if digits.is_empty() { "0".to_string() } else { digits.to_string() }, exp)
}

fn push_plain(out: &mut String, digits: &str, exp: i32) {
    if exp >= 0 {
        let int_len = (exp + 1) as usize;
        if digits.len() <= int_len {
            out.push_str(digits);
            for _ in digits.len()..int_len {
                out.push('0');
            }
            out.push_str(".0");
        } else {
            out.push_str(&digits[..int_len]);
            out.push('.');
            out.push_str(&digits[int_len..]);
        }
    } else {
        out.push_str("0.");
        for _ in 0..(-exp - 1) {
            out.push('0');
        }
        out.push_str(digits);
    }
}

fn layout_plain(sci: &str, negative: bool) -> String {
    let (digits, exp) = split_sci(sci);
    let mut out = String::with_capacity(digits.len() + 8);
    if negative {
        out.push('-');
    }
    push_plain(&mut out, &digits, exp);
    out
}

/// Java's two-significant-digit minimum: a one-digit shortest repr is replaced by the correctly
/// rounded two-digit one.
fn min_two_digits(shortest: String, two_digit: impl Fn() -> String) -> String {
    let mantissa = shortest.split('e').next().unwrap_or("");
    let digit_count = mantissa.chars().filter(|c| c.is_ascii_digit()).count();
    if digit_count <= 1 { two_digit() } else { shortest }
}

/// Lays out Rust's shortest scientific repr (`-1.2345e6`) the Java way.
fn layout(sci: &str, negative: bool) -> String {
    let (digits, exp) = split_sci(sci);
    let mut out = String::with_capacity(digits.len() + 8);
    if negative {
        out.push('-');
    }
    if (-3..7).contains(&exp) {
        push_plain(&mut out, &digits, exp);
    } else {
        out.push_str(&digits[..1]);
        out.push('.');
        if digits.len() > 1 {
            out.push_str(&digits[1..]);
        } else {
            out.push('0');
        }
        out.push('E');
        out.push_str(&exp.to_string());
    }
    out
}

fn strip_type_suffix(s: &str) -> &str {
    let t = s.trim();
    t.strip_suffix(['f', 'F', 'd', 'D']).unwrap_or(t)
}

/// `Float.parseFloat` for the forms the editor writes (decimal, scientific, NaN, Infinity).
pub fn parse_float(s: &str) -> Result<f32> {
    let t = strip_type_suffix(s);
    match t {
        "NaN" => Ok(f32::NAN),
        "Infinity" | "+Infinity" => Ok(f32::INFINITY),
        "-Infinity" => Ok(f32::NEG_INFINITY),
        _ => t.parse::<f32>().map_err(|_| crate::Error::new(format!("not a float: '{s}'"))),
    }
}

/// `Double.parseDouble` for the forms the editor writes.
pub fn parse_double(s: &str) -> Result<f64> {
    let t = strip_type_suffix(s);
    match t {
        "NaN" => Ok(f64::NAN),
        "Infinity" | "+Infinity" => Ok(f64::INFINITY),
        "-Infinity" => Ok(f64::NEG_INFINITY),
        _ => t.parse::<f64>().map_err(|_| crate::Error::new(format!("not a double: '{s}'"))),
    }
}

/// `Integer.parseInt` (decimal, optional sign).
pub fn parse_int<T: std::str::FromStr>(s: &str, what: &str) -> Result<T> {
    let t = s.trim();
    let t = t.strip_prefix('+').unwrap_or(t);
    match t.parse::<T>() {
        Ok(v) => Ok(v),
        Err(_) => bail!("not a {what}: '{s}'"),
    }
}

/// `Boolean.parseBoolean`: only the exact text "true" is true.
pub fn parse_bool(s: &str) -> bool {
    s == "true"
}

#[cfg(test)]
// exact binary values written in full: the ties under test
#[allow(clippy::excessive_precision)]
mod tests {
    use super::*;

    #[test]
    fn float_layout_matches_java() {
        let cases: &[(f32, &str)] = &[
            (1.0, "1.0"),
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (100.0, "100.0"),
            (0.5, "0.5"),
            (0.001, "0.001"),
            (0.0001, "1.0E-4"),
            (1234567.0, "1234567.0"),
            (10000000.0, "1.0E7"),
            (12345678.0, "1.2345678E7"),
            (-3.0e-5, "-3.0E-5"),
            (0.1, "0.1"),
            (1.23456, "1.23456"),
            (f32::MAX, "3.4028235E38"),
            (f32::MIN_POSITIVE, "1.1754944E-38"),
            (1.4e-45, "1.4E-45"),
            (3.0e-45, "2.8E-45"),
            (0.3, "0.3"),
            (7.0, "7.0"),
            (2133.03125, "2133.0312"),
            (9.5, "9.5"),
            (1.0e23, "1.0E23"),
        ];
        for (v, want) in cases {
            assert_eq!(float_to_string(*v), *want, "value {v}");
        }
    }

    #[test]
    fn double_layout_matches_java() {
        assert_eq!(double_to_string(1.0), "1.0");
        assert_eq!(double_to_string(0.1), "0.1");
        assert_eq!(double_to_string(1e7), "1.0E7");
        assert_eq!(double_to_string(123456.789), "123456.789");
        assert_eq!(double_to_string(1e-3), "0.001");
        assert_eq!(double_to_string(9.999e-4), "9.999E-4");
        assert_eq!(double_to_string(f64::MAX), "1.7976931348623157E308");
        assert_eq!(double_to_string(f64::from_bits(1)), "4.9E-324");
        assert_eq!(double_to_string(2133.03125), "2133.03125");
        assert_eq!(
            exact_decimal(decompose_f32(2133.03125).0, decompose_f32(2133.03125).1),
            ("213303125".to_string(), 3)
        );
    }

    #[test]
    fn plain_layout_never_uses_exponent() {
        assert_eq!(float_to_plain_string(2.4414063e-4), "0.00024414063"); // observed in 5.2 files
        assert_eq!(float_to_string(2.4414063e-4), "2.4414062E-4"); // JDK 27
        assert_eq!(float_to_plain_string(1781.0), "1781.0");
        assert_eq!(float_to_plain_string(1.0e7), "10000000.0");
        assert_eq!(float_to_plain_string(1.2345678e7), "12345678.0");
        assert_eq!(float_to_plain_string(-0.5), "-0.5");
        assert_eq!(double_to_plain_string(1e-7), "0.0000001");
    }

    #[test]
    fn parse_accepts_editor_forms() {
        assert_eq!(parse_float("1.0E-4").unwrap(), 1.0e-4);
        assert_eq!(parse_float("2.5f").unwrap(), 2.5);
        assert!(parse_float("NaN").unwrap().is_nan());
        assert_eq!(parse_float("-Infinity").unwrap(), f32::NEG_INFINITY);
        assert_eq!(parse_int::<i32>("-12", "int").unwrap(), -12);
        assert!(parse_int::<i32>("x", "int").is_err());
    }
}
