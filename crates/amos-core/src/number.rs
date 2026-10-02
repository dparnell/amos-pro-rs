//! Number conversions and AMOS number formatting.

/// Converts a Motorola Fast Floating Point value (as stored by AMOS in
/// tokenised programs and single precision variables) to an `f64`.
///
/// Layout: 24 bit normalised mantissa, sign bit, 7 bit excess-64 exponent.
pub fn ffp_to_f64(v: u32) -> f64 {
    if v & 0xFFFF_FF00 == 0 {
        return 0.0;
    }
    let mant = (v >> 8) as f64 / (1u64 << 24) as f64;
    let exp = (v & 0x7F) as i32 - 64;
    let val = mant * 2f64.powi(exp);
    if v & 0x80 != 0 { -val } else { val }
}

/// Converts an `f64` to Motorola Fast Floating Point (rounding to 24 bits).
pub fn f64_to_ffp(x: f64) -> u32 {
    if x == 0.0 || !x.is_finite() {
        return 0;
    }
    let sign = if x < 0.0 { 0x80 } else { 0 };
    let a = x.abs();
    let mut exp = a.log2().floor() as i32 + 1;
    let mut mant = (a / 2f64.powi(exp) * (1u64 << 24) as f64).round() as u64;
    if mant >= 1 << 24 {
        mant >>= 1;
        exp += 1;
    }
    if mant < 1 << 23 {
        mant <<= 1;
        exp -= 1;
    }
    let e = exp + 64;
    if e < 0 {
        return 0;
    }
    let e = e.min(127) as u32;
    ((mant as u32) << 8) | sign | e
}

/// Formats a float the way the AMOS lister does (`FloatToAsc` with 7
/// significant digits, trailing zeros removed, `.0` appended to integers).
pub fn format_float_listing(x: f64, digits: usize) -> String {
    let mut s = format_significant(x, digits);
    if !s.contains('.') && !s.contains('E') {
        s.push_str(".0");
    }
    s
}

/// `%.*G` style formatting with AMOS exponent style (` E+nn`).
pub fn format_significant(x: f64, digits: usize) -> String {
    if x == 0.0 {
        return "0".to_string();
    }
    let digits = digits.max(1);
    let exp = x.abs().log10().floor() as i32;
    // Round first, the exponent may change (9.9999999 -> 10).
    let sci = format!("{:.*e}", digits - 1, x);
    let (mant, e) = sci.split_once('e').unwrap();
    let e: i32 = e.parse().unwrap_or(exp);
    if e < -5 || e >= digits as i32 {
        let mut m = mant.to_string();
        if m.contains('.') {
            m = m.trim_end_matches('0').trim_end_matches('.').to_string();
        }
        let sign = if e < 0 { '-' } else { '+' };
        format!("{m} E{sign}{:02}", e.abs())
    } else {
        let decimals = (digits as i32 - 1 - e).max(0) as usize;
        let mut s = format!("{:.*}", decimals, x);
        if s.contains('.') {
            s = s.trim_end_matches('0').trim_end_matches('.').to_string();
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffp_roundtrip() {
        assert!((ffp_to_f64(0xCCCCCD3D) - 0.1).abs() < 1e-8);
        assert_eq!(ffp_to_f64(0xF8000044), 15.5);
        assert_eq!(f64_to_ffp(15.5), 0xF8000044);
        assert_eq!(f64_to_ffp(0.1), 0xCCCCCD3D);
        assert_eq!(f64_to_ffp(-1.0), 0x800000C1);
    }

    #[test]
    fn listing_format() {
        assert_eq!(format_float_listing(15.5, 7), "15.5");
        assert_eq!(format_float_listing(3.0, 7), "3.0");
        assert_eq!(format_float_listing(ffp_to_f64(0xCCCCCD3D), 7), "0.1");
    }
}
