//! Bit exact AMOS double precision text conversion (`Val`, Input and the
//! tokeniser's double constants).
//!
//! `AscToDouble` (`+Lib.s:27066`, the routine at `L3D9C0`) converts text with
//! the IEEE double routines of the C runtime linked into `+Lib.s` (after
//! `DoubleToAsc`), not with `mathieeedoubbas`: add `L3DE80`, multiply
//! `L3DFA0`, divide `L3E12A`, long -> double `L3DDA6`, compare `L3DDC6`,
//! negate `L3DE4E`, all ending in the normalise / round / pack routine
//! `L3E26A`. They are not correctly rounded: the multiplication leaves out
//! the low partial products and rounds twice, the division truncates its
//! quotient before rounding, the addition keeps a single sticky bit, and a
//! zero exponent field is zero. Here they are computed in closed form with
//! integers; `softdouble_reference.rs` (tests) keeps the instruction by
//! instruction transcription they are checked against, and the compiled
//! modules do the same arithmetic with wasm integer operations
//! (`amos-compiler`, `numfmt.rs`).
//!
//! A value is the `u64` of its bits (`D0` high, `D1` low). Mantissas are
//! handled as the routines do: the 53 bits at the top of a `u64` (`<< 11`).

/// The value `AscToDouble` returns on overflow (`L3DAE6`), positive.
pub const OVERFLOW: u64 = 0x43A9_9999_8D2C_7175;
pub const ONE: u64 = 0x3FF0_0000_0000_0000;
pub const TEN: u64 = 0x4024_0000_0000_0000;
const SIGN: u64 = 1 << 63;

#[inline]
fn efield(v: u64) -> u32 {
    ((v >> 52) & 0x7FF) as u32
}

/// The mantissa with its hidden bit, at the top of 64 bits.
#[inline]
fn m64(v: u64) -> u64 {
    ((v & 0x000F_FFFF_FFFF_FFFF) | 0x0010_0000_0000_0000) << 11
}

/// Sign byte of the routines (`SMI`): 0 or $FF.
#[inline]
fn sbyte(v: u64) -> u8 {
    if v & SIGN != 0 { 0xFF } else { 0 }
}

/// `L3E26A`: normalises the 64 bit mantissa `m` with exponent `e` (16 bit
/// word), rounds to 53 bits (above half, or half with an odd result: up)
/// and packs with the sign byte `s` (non zero: negative). Exponent below
/// the range: the smallest normal value; above: `overflow_value`.
pub fn pack(m: u64, e: u16, s: u8) -> u64 {
    if m == 0 {
        return 0;
    }
    let z = m.leading_zeros();
    let mut m = m << z;
    let mut e = e.wrapping_sub(z as u16);
    let low = m & 0x7FF;
    if low > 0x400 || (low == 0x400 && m & 0x800 != 0) {
        let (r, c) = m.overflowing_add(0x800);
        m = r;
        if c {
            m = (m >> 1) | SIGN;
            e = e.wrapping_add(1);
        }
    }
    let e = e.wrapping_add(0x3FF) as i16;
    let sign = if s != 0 { SIGN } else { 0 };
    if e < 0 {
        return sign | 0x0010_0000_0000_0000;
    }
    if e > 0x7FF {
        return overflow_value(s);
    }
    sign | ((e as u64) << 52) | ((m >> 11) & 0x000F_FFFF_FFFF_FFFF)
}

/// `L3E252`: all bits set but the sign (`D0` = $7FFFFFFF, `D1` = -1).
pub fn overflow_value(s: u8) -> u64 {
    (if s != 0 { SIGN } else { 0 }) | 0x7FFF_FFFF_FFFF_FFFF
}

/// `L3DDA6`: long -> double (exact).
pub fn from_long(v: i32) -> u64 {
    if v == 0 {
        return 0;
    }
    pack((v.unsigned_abs() as u64) << 32, 0x1F, if v < 0 { 0xFF } else { 0 })
}

/// `L3DE4E`: negation (zero exponent field: +0).
pub fn neg(v: u64) -> u64 {
    if efield(v) == 0 { 0 } else { v ^ SIGN }
}

/// `L3DDC6`: compares `a` with `b`: -1, 0 or 1 (sign and magnitude; a zero
/// exponent field is 0).
pub fn cmp(a: u64, b: u64) -> i32 {
    let a = if efield(a) == 0 { 0 } else { a };
    let b = if efield(b) == 0 { 0 } else { b };
    let o = match (a & SIGN != 0, b & SIGN != 0) {
        (false, false) => (a as i64).cmp(&(b as i64)),
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        // Both negative: the larger magnitude is the smaller.
        (true, true) => (b as i64).cmp(&(a as i64)),
    };
    o as i32
}

/// `L3DE80`: `a + b`. The smaller operand is shifted right with a sticky
/// bit (beyond 4 places) or dropped (from 55 places).
pub fn add(a: u64, b: u64) -> u64 {
    if efield(a) == 0 {
        return if efield(b) == 0 { 0 } else { b };
    }
    if efield(b) == 0 {
        return a;
    }
    let (mut m4, mut m5) = (m64(a), m64(b));
    let (mut e4, mut e5) = ((efield(a) as u16).wrapping_sub(0x3FF), (efield(b) as u16).wrapping_sub(0x3FF));
    let (mut s4, mut s5) = (sbyte(a), sbyte(b));
    if e4 != e5 {
        if (e4 as i16) < (e5 as i16) {
            std::mem::swap(&mut m4, &mut m5);
            std::mem::swap(&mut e4, &mut e5);
            std::mem::swap(&mut s4, &mut s5);
        }
        let n = e4.wrapping_sub(e5) as u32;
        m5 = if n >= 0x37 {
            0
        } else if n < 5 {
            m5 >> n
        } else {
            (m5 >> n) | ((m5 & ((1 << n) - 1) != 0) as u64)
        };
    }
    if s4 == s5 {
        let (r, c) = m4.overflowing_add(m5);
        if c {
            return pack((r >> 1) | SIGN, e4.wrapping_add(1), s4);
        }
        return pack(r, e4, s4);
    }
    // Different signs: the positive one minus the other.
    let (p, q) = if s4 != 0 { (m5, m4) } else { (m4, m5) };
    let (r, borrow) = p.overflowing_sub(q);
    if borrow { pack(r.wrapping_neg(), e4, 0xFF) } else { pack(r, e4, 0) }
}

/// `L3DFA0`: `a * b`. The 16 bit word products of weight 2^48 and more
/// (of the 128 bit product of the mantissas) are summed with the high
/// halves of those of weight 2^32 (the others are left out), the sum is
/// rounded at its bit 16 (half: a sticky bit), then by `pack`.
pub fn mul(a: u64, b: u64) -> u64 {
    if efield(a) == 0 || efield(b) == 0 {
        return 0;
    }
    let e = (efield(a) + efield(b)) as u16;
    let mut e = e.wrapping_sub(0x7FD);
    let mut s = sbyte(a) ^ sbyte(b);
    let (x, y) = (m64(a), m64(b));
    let w = |v: u64, i: u32| (v >> (16 * i)) & 0xFFFF;
    let p = |i: u32, j: u32| w(x, i) * w(y, j);
    let t6 = p(3, 3);
    let t5 = p(3, 2) + p(2, 3);
    let t4 = p(3, 1) + p(2, 2) + p(1, 3);
    let t3 = p(3, 0) + p(2, 1) + p(1, 2) + p(0, 3);
    let t2 = (p(2, 0) >> 16) + (p(1, 1) >> 16) + (p(0, 2) >> 16);
    let mut sum: u128 = ((t6 as u128) << 48) + ((t5 as u128) << 32) + ((t4 as u128) << 16) + (t3 + t2) as u128;
    if sum >> 64 > 0xFFFF {
        // (Never reached: the original increments the sign there.)
        s = s.wrapping_add(1);
        sum >>= 1;
    }
    let low = sum & 0xFFFF;
    if low == 0x8000 {
        sum |= 0x1_0000;
    } else if low > 0x8000 {
        sum += 0x1_0000;
        if sum >> 64 > 0xFFFF {
            e = e.wrapping_add(1);
            sum >>= 1;
        }
    }
    pack((sum >> 16) as u64, e, s)
}

/// `L3E12A`: `a / b`: the quotient of the mantissas truncated to 64 bits
/// (`DIVU.W` for a 16 bit divisor, else a non restoring division; the
/// same bits), then `pack`. Division by zero: `overflow_value`.
pub fn div(a: u64, b: u64) -> u64 {
    if efield(a) == 0 {
        return 0;
    }
    if efield(b) == 0 {
        return overflow_value(sbyte(a));
    }
    let d = ((efield(a) << 4) as u16).wrapping_sub((efield(b) << 4) as u16);
    let mut e = ((d as i16) >> 4).wrapping_sub(1) as u16;
    let (mut n, dv) = (m64(a), m64(b));
    if n >= dv {
        n >>= 1;
        e = e.wrapping_add(1);
    }
    let q = (((n as u128) << 64) / dv as u128) as u64;
    pack(q, e, sbyte(a) ^ sbyte(b))
}

/// Number of entries of `pow10_values`.
pub const POW10_LEN: usize = 400;

/// `L3DCFA`'s products: 10^n built by `n` multiplications of 1.0 by 10
/// (`mul(r, TEN)`), for n < `POW10_LEN`.
pub fn pow10_values() -> &'static [u64; POW10_LEN] {
    static T: std::sync::OnceLock<[u64; POW10_LEN]> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0; POW10_LEN];
        let mut v = ONE;
        for e in t.iter_mut() {
            *e = v;
            v = mul(v, TEN);
        }
        t
    })
}

/// `L3DCFA(n)`: 10^n as `AscToDouble` builds it (1.0 for n <= 0): the
/// table, then more multiplications (they reach a fixed point, the
/// overflow value).
pub fn pow10(n: i32) -> u64 {
    let t = pow10_values();
    if n <= 0 {
        return ONE;
    }
    if (n as usize) < POW10_LEN {
        return t[n as usize];
    }
    let mut v = t[POW10_LEN - 1];
    for _ in POW10_LEN as i32 - 1..n {
        let next = mul(v, TEN);
        if next == v {
            break;
        }
        v = next;
    }
    v
}

/// Integer digits summed exactly by `AscToDouble`: up to 15 digits, the
/// terms `digit * 10^k` (10^k with k <= 22 is exact, the multiplications
/// by 10 leave out no partial product) and the sums are integers below
/// 2^53, which the routines compute exactly.
pub const EXACT_DIGITS: usize = 15;

/// Places of the digit tables: `BuFloat` holds at most 33 characters.
pub const DIGIT_PLACES: usize = 33;

/// The terms `AscToDouble` adds for a digit `d` (1..9) at place `k` <
/// `DIGIT_PLACES`: `mul(pow10(k), d)` for integer digits (`[k][d - 1]`)
/// and `div(d, pow10(k + 1))` for fraction digits. (A 0 digit adds 0,
/// which leaves the sum as it is.)
pub fn digit_terms() -> &'static [[[u64; 9]; DIGIT_PLACES]; 2] {
    static T: std::sync::OnceLock<[[[u64; 9]; DIGIT_PLACES]; 2]> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = [[[0; 9]; DIGIT_PLACES]; 2];
        #[allow(clippy::needless_range_loop)]
        for k in 0..DIGIT_PLACES {
            for d in 1..10 {
                let (p, dv) = (pow10(k as i32), from_long(d as i32));
                t[0][k][d - 1] = mul(p, dv);
                t[1][k][d - 1] = div(dv, pow10(k as i32 + 1));
            }
        }
        t
    })
}

/// The text `ValRout` hands to `AscToDouble` / `AscToFloat` (`BuFloat`,
/// `+ILib.s` `Ca1`): the characters other than spaces of `s` (from the
/// sign, if any, to the end of the number), at most 33.
pub fn bufloat(s: &[u8]) -> Vec<u8> {
    s.iter().copied().filter(|&c| c != b' ').take(33).collect()
}

/// Character class bits of the C runtime table (`DDebut+$91`): 4 digit,
/// $10 space. (Texts from `ValRout` have no characters >= $80, which
/// would index the bytes before the table.)
fn ctype(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => 0x0C,
        9..=13 => 0x30,
        b' ' => 0x90,
        _ => 0,
    }
}

fn is_digit(c: u8) -> bool {
    ctype(c) & 4 != 0
}

/// `AscToDouble` (`L3D9C0`): text to the bits of a double. Spaces, a sign,
/// then the integer digits summed from the last (`digit * 10^k`, stopping
/// with the overflow value if the sum decreases), the fraction digits
/// (`digit / 10^k`), and an exponent (`e`/`E`, sign, digits: the value
/// multiplied or divided by 10^n). Anything else ends the number.
pub fn asc_to_double(s: &[u8]) -> u64 {
    let at = |i: usize| s.get(i).copied().unwrap_or(0);
    let mut i = 0;
    while ctype(at(i)) & 0x10 != 0 {
        i += 1;
    }
    let mut neg = false;
    match at(i) {
        b'+' => i += 1,
        b'-' => {
            neg = true;
            i += 1;
        }
        c if is_digit(c) || c == b'.' => {}
        _ => return 0,
    }
    let start = i;
    let mut end = start;
    while is_digit(at(end)) {
        end += 1;
    }
    let terms = digit_terms();
    let mut v: u64 = 0;
    if end - start <= EXACT_DIGITS {
        // Every term and partial sum is an integer below 2^53: exact, so
        // the sum is the integer itself.
        let n = s[start..end].iter().fold(0u64, |n, &c| n * 10 + (c - b'0') as u64);
        v = (n as f64).to_bits();
    }
    for p in (start..end).rev().filter(|_| end - start > EXACT_DIGITS) {
        let (k, d) = (end - p - 1, s[p] - b'0');
        if d == 0 {
            continue;
        }
        let old = v;
        let t = if k < DIGIT_PLACES { terms[0][k][d as usize - 1] } else { mul(pow10(k as i32), from_long(d as i32)) };
        v = add(t, v);
        if cmp(v, old) < 0 {
            return overflow(neg);
        }
    }
    let mut q = end;
    if at(q) == b'.' {
        q += 1;
        while is_digit(at(q)) {
            let (k, d) = (q - end, s[q] - b'0');
            if d != 0 {
                let t = if k <= DIGIT_PLACES {
                    terms[1][k - 1][d as usize - 1]
                } else {
                    div(from_long(d as i32), pow10(k as i32))
                };
                v = add(t, v);
            }
            q += 1;
        }
    }
    if at(q) == b'e' || at(q) == b'E' {
        let mut e = q;
        if at(q + 1) == b'-' || at(q + 1) == b'+' {
            e += 1;
        }
        let eneg = at(e) == b'-';
        let mut d = e + 1;
        if is_digit(at(d)) {
            let mut n: i32 = 0;
            while is_digit(at(d)) {
                let old = n;
                n = n.wrapping_mul(10).wrapping_add(at(d) as i32 - 0x30);
                if n < old {
                    if !eneg {
                        return overflow(neg);
                    }
                    v = 0;
                    n = 0;
                    break;
                }
                d += 1;
            }
            if eneg {
                v = div(v, pow10(n));
            } else {
                let old = v;
                v = mul(pow10(n), v);
                if cmp(v, old) < 0 {
                    return overflow(neg);
                }
            }
        }
    }
    if neg { self::neg(v) } else { v }
}

/// `L3DAE6`: the overflow value with the sign of the text.
fn overflow(neg: bool) -> u64 {
    if neg { OVERFLOW | SIGN } else { OVERFLOW }
}

/// `AscToDouble` as an `f64`.
pub fn asc_to_f64(s: &[u8]) -> f64 {
    f64::from_bits(asc_to_double(s))
}

#[cfg(test)]
#[path = "softdouble_reference.rs"]
mod reference;

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// Debug builds run fewer iterations.
    fn count(n: usize) -> usize {
        if cfg!(debug_assertions) { n / 10 } else { n }
    }

    /// Random operands: normal values in a range, extremes, zero exponent
    /// fields, the overflow value, raw words.
    fn operand(r: &mut Rng) -> u64 {
        let v = r.next();
        match r.below(8) {
            0 => v,
            1 => (v & 0x800F_FFFF_FFFF_FFFF) | ((r.below(4) | ((0x7FC + r.below(4)) * r.below(2))) << 52),
            2 => [0, SIGN, ONE, TEN, OVERFLOW, overflow_value(0), overflow_value(0xFF), 0x0010_0000_0000_0000]
                [r.below(8) as usize],
            3 => from_long(r.next() as i32 >> r.below(31)),
            4 => pow10(r.below(330) as i32),
            _ => (v & 0x800F_FFFF_FFFF_FFFF) | ((0x3FF - 70 + r.below(140)) << 52),
        }
    }

    #[test]
    fn operations_match_the_transcription() {
        let mut r = Rng(0x0DDB_A11C_0FFE_E123);
        for _ in 0..count(1_000_000) {
            let (a, b) = (operand(&mut r), operand(&mut r));
            assert_eq!(add(a, b), reference::add(a, b), "add {a:016X} {b:016X}");
            assert_eq!(mul(a, b), reference::mul(a, b), "mul {a:016X} {b:016X}");
            assert_eq!(div(a, b), reference::div(a, b), "div {a:016X} {b:016X}");
            assert_eq!(cmp(a, b), reference::cmp(a, b), "cmp {a:016X} {b:016X}");
            assert_eq!(neg(a), reference::neg(a), "neg {a:016X}");
        }
        for v in (-70_000i32..70_000).chain([i32::MIN, i32::MAX, i32::MIN + 1]) {
            assert_eq!(from_long(v), reference::from_long(v), "{v}");
            assert_eq!(from_long(v), (v as f64).to_bits(), "{v}");
        }
        // Same exponents and close ones, both signs (the alignment paths).
        for _ in 0..count(1_000_000) {
            let a = operand(&mut r);
            let shift = r.below(70);
            let b = (r.next() & 0x800F_FFFF_FFFF_FFFF) | ((a >> 52 & 0x7FF).saturating_sub(shift) << 52);
            assert_eq!(add(a, b), reference::add(a, b), "add {a:016X} {b:016X}");
            assert_eq!(add(b, a), reference::add(b, a), "add {b:016X} {a:016X}");
            // Divisors with 16 significant bits (DIVU.W).
            let d = (r.next() & 0x800F_0000_0000_0000) | (r.below(0x7FE) + 1) << 52;
            assert_eq!(div(a, d), reference::div(a, d), "div {a:016X} {d:016X}");
            assert_eq!(div(d, d), reference::div(d, d), "div {d:016X} {d:016X}");
        }
    }

    #[test]
    fn powers_of_ten_are_the_loop() {
        let mut v = ONE;
        for n in 0..3000 {
            assert_eq!(pow10(n), v, "10^{n}");
            v = mul(v, TEN);
        }
        // The overflow value is a fixed point of the loop.
        assert_eq!(pow10(5000), overflow_value(0));
        assert_eq!(mul(overflow_value(0), TEN), overflow_value(0));
        assert_eq!(pow10(-3), ONE);
    }

    fn random_text(r: &mut Rng) -> Vec<u8> {
        let set = b"0123456789.eE+- \t-x";
        match r.below(5) {
            0 => (0..r.below(40)).map(|_| set[r.below(set.len() as u64) as usize]).collect(),
            1 => {
                let mut t = String::new();
                if r.below(3) == 0 {
                    t.push_str(["-", "+", " ", "\t-"][r.below(4) as usize]);
                }
                for _ in 0..r.below(40) {
                    t.push((b'0' + r.below(10) as u8) as char);
                }
                if r.below(2) == 0 {
                    t.push('.');
                    for _ in 0..r.below(40) {
                        t.push((b'0' + r.below(10) as u8) as char);
                    }
                }
                if r.below(2) == 0 {
                    t.push_str(["e", "E", "e-", "e+", "E-"][r.below(5) as usize]);
                    let digits = match r.below(4) {
                        0 => r.below(12),
                        _ => r.below(4),
                    };
                    for _ in 0..digits {
                        t.push((b'0' + r.below(10) as u8) as char);
                    }
                }
                t.into_bytes()
            }
            2 => format!("{:e}", f64::from_bits(r.next() & 0x7FEF_FFFF_FFFF_FFFF)).into_bytes(),
            3 => format!("{}", f64::from_bits(r.next() & 0x7FEF_FFFF_FFFF_FFFF)).into_bytes(),
            _ => format!("{}.{}", r.below(1_000_000), r.below(1_000_000)).into_bytes(),
        }
    }

    /// How `AscToDouble` differs from a correctly rounded conversion (the
    /// interpreter's earlier behaviour): `cargo test --release -p amos-core
    /// survey_vs_correctly_rounded -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn survey_vs_correctly_rounded() {
        let mut r = Rng(0x5EED_1234_ABCD_0042);
        let digits = |r: &mut Rng, n: u64| -> String { (0..n).map(|_| (b'0' + r.below(10) as u8) as char).collect() };
        type Gen = Box<dyn Fn(&mut Rng) -> String>;
        let cats: Vec<(&str, Gen)> = vec![
            (
                "integers, 1-15 digits",
                Box::new(move |r| {
                    let k = 1 + r.below(15);
                    digits(r, k)
                }),
            ),
            (
                "short decimals (1-6 . 1-6 digits)",
                Box::new(move |r| {
                    let (i, f) = (1 + r.below(6), 1 + r.below(6));
                    format!("{}.{}", digits(r, i), digits(r, f))
                }),
            ),
            (
                "2 decimals (prices)",
                Box::new(move |r| {
                    let i = r.below(100000);
                    format!("{i}.{}", digits(r, 2))
                }),
            ),
            (
                "15-17 significant digits",
                Box::new(move |r| {
                    let (i, f) = (1 + r.below(8), 9 + r.below(8));
                    format!("{}.{}", digits(r, i), digits(r, f))
                }),
            ),
            (
                "with exponent",
                Box::new(move |r| {
                    let (f, e) = (1 + r.below(6), r.below(80) as i64 - 40);
                    format!("{}.{}e{e}", digits(r, 1), digits(r, f))
                }),
            ),
        ];
        let n = 1_000_000;
        // The editor lists a constant (15 digits) and tokenises it again.
        for (name, g) in &cats {
            let mut unstable = 0;
            for _ in 0..n {
                let b = asc_to_double(g(&mut r).as_bytes());
                let text = crate::ffp::format_double_listing(f64::from_bits(b));
                let c = asc_to_double(text.to_lowercase().as_bytes());
                if c != b {
                    unstable += 1;
                    let b2 = crate::tokenise::parse_float_text(&text.to_lowercase()).to_bits();
                    if unstable < 3 {
                        println!("  {text}: {b:016X} -> {c:016X} (correctly rounded {b2:016X})");
                    }
                }
            }
            println!("{name}: listing round trip changes {:.3}%", unstable as f64 * 100.0 / n as f64);
        }
        for (name, g) in &cats {
            let (mut diff, mut shown, mut max_ulp) = (0, 0, 0u64);
            let mut ex = Vec::new();
            for _ in 0..n {
                let t = g(&mut r);
                let a = asc_to_double(t.as_bytes());
                let b = crate::tokenise::parse_float_text(&t).to_bits();
                if a != b {
                    diff += 1;
                    max_ulp = max_ulp.max(a.abs_diff(b));
                    let (fa, fb) = (f64::from_bits(a), f64::from_bits(b));
                    let free = crate::ffp::Fix::Free;
                    if crate::ffp::format_double(fa, free) != crate::ffp::format_double(fb, free) {
                        shown += 1;
                    }
                    if ex.len() < 4 {
                        ex.push(format!("{t} -> {fa:?} (was {fb:?})"));
                    }
                }
            }
            println!(
                "{name}: {:.2}% differ (max {max_ulp} ulp), {:.3}% print differently; e.g. {ex:?}",
                diff as f64 * 100.0 / n as f64,
                shown as f64 * 100.0 / n as f64
            );
        }
    }

    #[test]
    fn conversion_matches_the_transcription() {
        let mut r = Rng(0xFACE_B00C_1234_5678);
        let fixed: Vec<&[u8]> = vec![
            b"",
            b" ",
            b"-",
            b"+",
            b".",
            b"-.",
            b"e5",
            b"1e",
            b"1e+",
            b"1e-",
            b"1.e5",
            b".5e-3",
            b"0",
            b"-0",
            b"-0.0",
            b"1e308",
            b"1e309",
            b"1.8e308",
            b"1e-308",
            b"1e-330",
            b"1e-400",
            b"1e400",
            b"-1e400",
            b"1e99999999999",
            b"1e-99999999999",
            b"2147483647e0",
            b"1e2147483647",
            b"1e2147483648",
            b"1e-2147483648",
            b"179769313486231570000000000000000000000",
            b"0.86",
            b"123.456",
            b"9007199254740993",
            b"00000000000000000000000000000001",
            b"1x5",
            b"--1",
            b"+-1",
            b" \t 12",
            b"1 2",
        ];
        for t in fixed {
            assert_eq!(asc_to_double(t), reference::asc_to_double(t), "{:?}", String::from_utf8_lossy(t));
        }
        for _ in 0..count(200_000) {
            let t = random_text(&mut r);
            assert_eq!(asc_to_double(&t), reference::asc_to_double(&t), "{:?}", String::from_utf8_lossy(&t));
        }
    }
}
