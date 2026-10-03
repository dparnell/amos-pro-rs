//! Bit exact AMOS single precision arithmetic (Motorola Fast Floating Point)
//! and AMOS number <-> text conversion.
//!
//! FFP layout: bits 31..8 mantissa (normalised, bit 31 set), bit 7 sign,
//! bits 6..0 exponent (excess 64). Zero is `$00000000`.
//!
//! The arithmetic routines are literal ports of the Motorola FFP code that
//! AMOS carries in `+Lib.s` for its own conversions (`L28422` add, `L28410`
//! sub, `L2858A` mul, `L28518` div, `L283E4` cmp, `L28270` long->FFP,
//! `L28300` FFP->long). Registers are emulated with `u32` and byte
//! operations so the rounding (and the quirks) match the 68000 code. At run
//! time AMOS uses `mathffp.library` (`SPAdd`, ...) from ROM for program
//! arithmetic; that library is built from the same Motorola FFP sources, so
//! the same routines are used for both here.
//!
//! The text routines port `a2ffp` (`+Lib.s:26774`, behind `L_AscToFloat`),
//! `ffp2a` (`+Lib.s:26226`), `F2a` (`+Lib.s:25976`), `FloatToAsc`
//! (`+Lib.s:25923`), `LongToAsc` (`+Lib.s:25671`) and the double precision
//! `Dtoa` (`+Lib.s:27080`, via `Float2AsciiD`, `+ILib.s:7684`).

use std::cmp::Ordering;

/// A Motorola Fast Floating Point value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Ffp(pub u32);

const ONE: u32 = 0x8000_0041;
const TWO: u32 = 0x8000_0042;
const HALF: u32 = 0x8000_0040;
const TEN: u32 = 0xA000_0044;
const TWO24: u32 = 0x8000_0059;

#[inline]
const fn lb(r: u32) -> u8 {
    r as u8
}

#[inline]
const fn setb(r: u32, b: u8) -> u32 {
    (r & !0xFF) | b as u32
}

// ---------------------------------------------------------------------------
// Add / subtract (L28422 / L28410)
// ---------------------------------------------------------------------------

/// `L28422`: d7 = d7 + d6.
fn ffp_add(d7: u32, d6: u32) -> u32 {
    add_core(d7, d6, lb(d6))
}

/// `L28410`: d7 = d7 - d6. Identical to adding with the sign of d6 flipped
/// (the low byte of d6 is otherwise only used cleared or cancelled).
fn ffp_sub(d7: u32, d6: u32) -> u32 {
    let d4b = lb(d6);
    if d4b == 0 {
        return d7;
    }
    add_core(d7, d6, d4b ^ 0x80)
}

/// Add with `d4b` the effective sign/exponent byte of d6.
fn add_core(d7: u32, d6: u32, d4b: u8) -> u32 {
    if d4b & 0x80 != 0 {
        // L28484
        let d5b = lb(d7);
        if d5b & 0x80 != 0 {
            return add_same(d7, d6, d4b, d5b);
        }
        if d5b == 0 {
            return setb(d6, d4b);
        }
        return add_diff(d7, d6, d4b, d5b);
    }
    if d4b == 0 {
        return d7; // L28466
    }
    let d5b = lb(d7);
    if d5b & 0x80 != 0 {
        return add_diff(d7, d6, d4b, d5b);
    }
    if d5b == 0 {
        return setb(d6, d4b); // L28460
    }
    add_same(d7, d6, d4b, d5b)
}

/// `L2844C`: carry out of the mantissa addition.
fn add_carry(sum: u32, d4b: u8) -> u32 {
    let d7 = (sum >> 1) | 0x8000_0000; // ROXR with X=1
    let (n, c) = d4b.overflowing_add(1);
    let v = (d4b as i8).checked_add(1).is_none();
    if v || c {
        // L28454: overflow, largest magnitude with the sign.
        return 0xFFFF_FF00 | d4b as u32;
    }
    setb(d7, n)
}

/// `L2842E`: operands of the same sign.
fn add_same(d7: u32, d6: u32, d4b: u8, d5b: u8) -> u32 {
    let d5 = d5b.wrapping_sub(d4b);
    if (d5 as i8) < 0 {
        // L2846A: d6 has the larger exponent.
        if (d5 as i8) <= -24 {
            return setb(d6, d4b);
        }
        let n = (d5 as i8).wrapping_neg() as u32;
        let d3 = setb(d6, 0x80);
        let x = (d7 & !0xFF) >> n;
        let (s, c) = x.overflowing_add(d3);
        if c {
            return add_carry(s, d4b);
        }
        return setb(s, d4b);
    }
    let d4b = lb(d7);
    if d5 >= 24 {
        return d7;
    }
    let d3 = (d6 & !0xFF) >> d5;
    let x = setb(d7, 0x80);
    let (s, c) = x.overflowing_add(d3);
    if c {
        return add_carry(s, d4b);
    }
    setb(s, d4b)
}

/// `L2848A`: operands of different signs.
fn add_diff(d7: u32, d6: u32, d4b: u8, d5b_x: u8) -> u32 {
    let d5 = (d5b_x ^ 0x80).wrapping_sub(d4b);
    if d5 == 0 {
        // L284E2: equal exponents.
        let xb = lb(d7);
        let yb = d4b;
        let diff = setb(d7, lb(d6)).wrapping_sub(d6);
        if diff == 0 {
            return 0;
        }
        if (diff as i32) > 0 {
            return normalise_sub(diff, xb, xb);
        }
        return normalise_sub(diff.wrapping_neg(), yb, yb);
    }
    let (big, small, n, d4b) = if (d5 as i8) < 0 {
        // L284D0
        if (d5 as i8) <= -24 {
            return setb(d6, d4b);
        }
        (setb(d6, 0x80), d7, (d5 as i8).wrapping_neg() as u32, d4b)
    } else {
        if d5 >= 24 {
            return d7;
        }
        (setb(d7, 0x80), d6, d5 as u32, lb(d7))
    };
    // L284A0
    let d3 = (small & !0xFF) >> n;
    let r = big.wrapping_sub(d3);
    if r & 0x8000_0000 != 0 {
        return setb(r, d4b);
    }
    normalise_sub(r, d4b, d4b)
}

/// `L284AA`: normalise after a subtraction (d5b holds the original byte to
/// detect exponent underflow through a sign change).
fn normalise_sub(d7: u32, d4b: u8, d5b: u8) -> u32 {
    let mut d7 = d7 & !0xFF;
    let mut d4b = d4b.wrapping_sub(1);
    if d7 <= 0x7FFF {
        d7 = d7.rotate_left(16);
        d4b = d4b.wrapping_sub(0x10);
    }
    // ADD.L D7,D7 / DBMI D4: decrement for every shift that does not
    // normalise.
    for _ in 0..40 {
        d7 = d7.wrapping_add(d7);
        if d7 & 0x8000_0000 != 0 || d7 == 0 {
            break;
        }
        d4b = d4b.wrapping_sub(1);
    }
    if (d4b ^ d5b) & 0x80 != 0 {
        return 0;
    }
    if d4b == 0 {
        return 0;
    }
    setb(d7, d4b)
}

// ---------------------------------------------------------------------------
// Multiply (L2858A)
// ---------------------------------------------------------------------------

const fn ffp_mul(x: u32, y: u32) -> u32 {
    let d5b = lb(x);
    if d5b == 0 {
        return x;
    }
    let d4b = lb(y);
    if d4b == 0 {
        return 0;
    }
    let a = ((d5b << 1) ^ 0x80) as i8;
    let b = ((d4b << 1) ^ 0x80) as i8;
    let (sum, ov) = a.overflowing_add(b);
    if ov {
        if sum >= 0 {
            return 0; // underflow
        }
        return setb(x, lb(x) ^ lb(y)) | 0xFFFF_FF7F;
    }
    let mut eb = ((d5b ^ d4b) & 0x80) | (((sum as u8) ^ 0x80) >> 1);
    let xh = x >> 16;
    let xl = x & 0xFF00;
    let yh = y >> 16;
    let yl = y & 0xFF00;
    let mut d4 = (yl * xl).rotate_left(16);
    d4 = d4.wrapping_add(xh * yl);
    let (s, c) = d4.overflowing_add(yh * xl);
    let d4 = ((c as u32) << 16) | (s >> 16);
    let mut d7 = (xh * yh).wrapping_add(d4);
    if d7 & 0x8000_0000 != 0 {
        d7 = d7.wrapping_add(0x80);
        if eb == 0 {
            return 0;
        }
        return setb(d7, eb);
    }
    // L285E2
    if eb == 0x80 || eb == 0x00 {
        return 0;
    }
    eb = eb.wrapping_sub(1);
    d7 = d7.wrapping_add(0x40);
    let c = d7 & 0x8000_0000 != 0;
    d7 = d7.wrapping_add(d7);
    if c {
        d7 = (d7 >> 1) | 0x8000_0000;
        eb = eb.wrapping_add(1);
    }
    if eb == 0 {
        return 0;
    }
    setb(d7, eb)
}

// ---------------------------------------------------------------------------
// Divide (L28518)
// ---------------------------------------------------------------------------

/// DIVU.W semantics: `None` on overflow (destination unchanged).
const fn divu(d: u32, s: u32) -> Option<u32> {
    let s = s & 0xFFFF;
    let q = d / s;
    if q > 0xFFFF {
        return None;
    }
    Some(((d % s) << 16) | q)
}

const fn ffp_div(x: u32, y: u32) -> u32 {
    let d5b = lb(y);
    if d5b == 0 {
        // Division by zero traps in the original; callers check first.
        return 0xFFFF_FF7F | ((lb(x) ^ d5b) & 0x80) as u32;
    }
    if x == 0 {
        return 0;
    }
    let a = ((lb(x) << 1) ^ 0x80) as i8;
    let b = ((d5b << 1) ^ 0x80) as i8;
    let (diff, ov) = a.overflowing_sub(b);
    if ov {
        if diff < 0 {
            return setb(x, lb(x) ^ d5b) | 0xFFFF_FF7F;
        }
        return 0;
    }
    let mut d4b = diff as u8;
    let mut m = x & !0xFF;
    let xh = m >> 16;
    let yh = y >> 16;
    if (xh.wrapping_sub(yh) & 0x8000) == 0 {
        if (d4b as i8).checked_add(2).is_none() {
            // L2850A: overflow; the sign comes from the divisor only.
            return d5b as u32 | 0xFFFF_FF7F;
        }
        d4b = d4b.wrapping_add(2);
        m >>= 1;
    }
    let mut eb = ((lb(x) ^ d5b) & 0x80) | ((d4b ^ 0x80) >> 1);
    // First 16 quotient bits.
    let (q1, rem_hi) = match divu(m, yh) {
        Some(r) => (r & 0xFFFF, m.wrapping_sub((r & 0xFFFF) * yh)),
        None => (m & 0xFFFF, m.wrapping_sub((m & 0xFFFF) * yh)),
    };
    let mut q1 = q1;
    let mut d7 = rem_hi.rotate_left(16);
    let d3 = (y & 0xFF00) * q1;
    let (r, borrow) = d7.overflowing_sub(d3);
    d7 = r;
    if borrow {
        d7 = d7.wrapping_add(y & !0xFF);
        q1 = q1.wrapping_sub(1) & 0xFFFF;
    }
    // L28566
    d7 &= 0xFFFF_0000;
    if let Some(r) = divu(d7, yh) {
        d7 = r;
    }
    let mut d5 = (q1 << 16) | (d7 & 0xFFFF);
    if q1 & 0x8000 == 0 {
        d5 = d5.wrapping_add(d5);
        eb = eb.wrapping_sub(1);
    }
    d5 = d5.wrapping_add(0x80);
    if eb == 0 {
        return 0;
    }
    setb(d5, eb)
}

// ---------------------------------------------------------------------------
// Compare, negate, conversions
// ---------------------------------------------------------------------------

/// `L283E4`: signed comparison of x against y as the 68000 flags give it.
fn ffp_cmp(x: u32, y: u32) -> Ordering {
    let xb = lb(x) as i8;
    let yb = lb(y) as i8;
    if xb < 0 && yb < 0 {
        if xb != yb {
            return yb.cmp(&xb);
        }
        return (y as i32).cmp(&(x as i32));
    }
    if xb != yb {
        return xb.cmp(&yb);
    }
    (x as i32).cmp(&(y as i32))
}

/// `L28406`.
fn ffp_neg(x: u32) -> u32 {
    if lb(x) == 0 { x } else { x ^ 0x80 }
}

/// `L28270`: long -> FFP (low bits are truncated).
fn ffp_from_long(v: i32) -> u32 {
    if v == i32::MIN {
        // The original loops forever here; this is the exact value.
        return 0x8000_00E0;
    }
    let neg = v < 0;
    let mut v = v.wrapping_abs();
    if v == 0 {
        return 0;
    }
    let mut e: i32 = 24;
    while v & 0x7F00_0000 != 0 {
        v >>= 1;
        e += 1;
    }
    while v & 0x0080_0000 == 0 {
        v <<= 1;
        e -= 1;
    }
    let mut r = ((v as u32) << 8) | ((e + 0x40) as u32 & 0x7F);
    if neg {
        r |= 0x80;
    }
    r
}

/// `L28300`: FFP -> long, truncating toward zero, saturating.
fn ffp_to_long(x: u32) -> i32 {
    let e = (x & 0x7F) as i32 - 0x40;
    if x == 0 || e < 0 {
        return 0;
    }
    let neg = x & 0x80 != 0;
    if e > 31 {
        return if neg { i32::MIN } else { i32::MAX };
    }
    let mut m = ((x as i32) >> 8) & 0xFF_FFFF;
    let sh = e - 24;
    if sh < 0 {
        m >>= -sh;
    } else {
        m <<= sh;
    }
    if neg { m.wrapping_neg() } else { m }
}

#[allow(clippy::should_implement_trait)]
impl Ffp {
    pub const ZERO: Ffp = Ffp(0);
    pub const ONE: Ffp = Ffp(ONE);
    pub const TEN: Ffp = Ffp(TEN);

    /// `SPFlt`: integer to FFP (bits below the 24 bit mantissa are truncated).
    pub fn from_i32(v: i32) -> Ffp {
        Ffp(ffp_from_long(v))
    }

    /// `SPFix`: truncates toward zero, saturates to `i32::MIN`/`i32::MAX`.
    pub fn to_i32(self) -> i32 {
        ffp_to_long(self.0)
    }

    /// Exact value.
    pub fn to_f64(self) -> f64 {
        let v = self.0;
        if v & 0x7F == 0 && v & 0x80 == 0 {
            return 0.0;
        }
        // mant / 2^24 * 2^(e - 64), exact: built from the bits instead of
        // `powi` (same value, it is on the interpreter's hot path).
        let val = (v >> 8) as f64 * crate::number::pow2((v & 0x7F) as i32 - 64 - 24);
        if v & 0x80 != 0 { -val } else { val }
    }

    /// Converts an `f64` (e.g. the result of a transcendental function) to
    /// FFP. The mantissa is **truncated** to 24 bits, as AMOS does when it
    /// converts doubles to single precision (`Dp2Sp` + `Ieee2FFP`, both
    /// truncating). Overflow saturates to the largest FFP, underflow gives 0.
    pub fn from_f64(v: f64) -> Ffp {
        if v == 0.0 || v.is_nan() {
            return Ffp::ZERO;
        }
        let sign = if v < 0.0 { 0x80 } else { 0 };
        if v.is_infinite() {
            return Ffp(0xFFFF_FF7F | sign);
        }
        let bits = v.abs().to_bits();
        let bexp = ((bits >> 52) & 0x7FF) as i32;
        if bexp == 0 {
            return Ffp::ZERO;
        }
        // value = 1.f * 2^(bexp-1023) = 0.1f * 2^(bexp-1022)
        let e = bexp - 1022 + 64;
        if e <= 0 {
            return Ffp::ZERO;
        }
        if e > 127 {
            return Ffp(0xFFFF_FF7F | sign);
        }
        let mant = ((bits | (1u64 << 52)) >> 29) as u32; // 24 bits
        Ffp((mant << 8) | sign | e as u32)
    }

    pub fn add(self, o: Ffp) -> Ffp {
        Ffp(ffp_add(self.0, o.0))
    }
    pub fn sub(self, o: Ffp) -> Ffp {
        Ffp(ffp_sub(self.0, o.0))
    }
    pub fn mul(self, o: Ffp) -> Ffp {
        Ffp(ffp_mul(self.0, o.0))
    }
    /// Division; division by zero must be handled by the caller.
    pub fn div(self, o: Ffp) -> Ffp {
        Ffp(ffp_div(self.0, o.0))
    }
    pub fn neg(self) -> Ffp {
        Ffp(ffp_neg(self.0))
    }
    pub fn abs(self) -> Ffp {
        Ffp(self.0 & !0x80)
    }
    pub fn cmp(self, o: Ffp) -> Ordering {
        ffp_cmp(self.0, o.0)
    }
    pub fn is_zero(self) -> bool {
        lb(self.0) == 0
    }
    pub fn is_negative(self) -> bool {
        self.0 & 0x80 != 0
    }

    /// `SPFloor`: largest integer not greater than the value (exact).
    pub fn floor(self) -> Ffp {
        let v = self.0;
        if self.is_zero() {
            return Ffp::ZERO;
        }
        let neg = v & 0x80 != 0;
        let e = (v & 0x7F) as i32 - 64;
        if e <= 0 {
            return if neg { Ffp(ONE | 0x80) } else { Ffp::ZERO };
        }
        if e >= 24 {
            return self;
        }
        let mant = v >> 8;
        let frac_mask = (1u32 << (24 - e)) - 1;
        let mut int = mant & !frac_mask;
        let mut exp = e;
        if neg && mant & frac_mask != 0 {
            int += 1 << (24 - e);
            if int >= 1 << 24 {
                int >>= 1;
                exp += 1;
            }
        }
        Ffp((int << 8) | (v & 0x80) | (exp + 64) as u32)
    }
}

// ---------------------------------------------------------------------------
// Tables of the conversions
// ---------------------------------------------------------------------------

/// Entries of the power of ten tables.
pub const POW10_LEN: usize = 48;

const fn pow10_table(up: bool) -> [u32; POW10_LEN] {
    let mut t = [0; POW10_LEN];
    let mut v = ONE;
    let mut k = 0;
    while k < POW10_LEN {
        t[k] = v;
        v = if up { ffp_mul(v, TEN) } else { ffp_div(v, TEN) };
        k += 1;
    }
    t
}

const fn half_table() -> [u32; POW10_LEN] {
    let mut t = [0; POW10_LEN];
    let mut k = 0;
    while k < POW10_LEN {
        t[k] = ffp_div(POW10_DOWN[k], TWO);
        k += 1;
    }
    t
}

/// `1.0` multiplied `k` times by 10 in FFP (`POW10_UP[k]`), divided `k`
/// times by 10 (`POW10_DOWN[k]`), and that divided by 2 (`HALF_DOWN[k]`):
/// the values `a2ffp` and `ffp2a` compute with loops that do not depend on
/// their input, so a table of the same FFP results gives identical values.
pub const POW10_UP: [u32; POW10_LEN] = pow10_table(true);
pub const POW10_DOWN: [u32; POW10_LEN] = pow10_table(false);
pub const HALF_DOWN: [u32; POW10_LEN] = half_table();

/// `1.0` multiplied (`up`) or divided `k` times by 10, as the loops do.
fn pow10(k: u32, up: bool) -> u32 {
    let t = if up { &POW10_UP } else { &POW10_DOWN };
    if (k as usize) < POW10_LEN {
        return t[k as usize];
    }
    let mut v = t[POW10_LEN - 1];
    for _ in POW10_LEN as u32 - 1..k {
        v = if up { ffp_mul(v, TEN) } else { ffp_div(v, TEN) };
    }
    v
}

/// Largest integer accumulated exactly by `a2ffp`'s digit loop: below 2^24
/// `v * 10 + digit` is exact in FFP (the mantissa holds it), so integer
/// arithmetic gives the same value.
pub const EXACT_INT: u32 = 1 << 24;

// ---------------------------------------------------------------------------
// Text -> FFP (a2ffp)
// ---------------------------------------------------------------------------

/// `a2ffp` (`+Lib.s:26774`, the routine behind `L_AscToFloat`): converts text
/// like `"179.35"` or `"-1.5e10"` to FFP, bit exact (including its rounding:
/// the digits are accumulated in FFP and multiplied by a power of ten built
/// by repeated FFP multiplication or division by 10).
pub fn ascii_to_ffp(text: &[u8]) -> Ffp {
    let at = |i: usize| text.get(i).copied().unwrap_or(0);
    let mut i = 0;
    while at(i) == b' ' || at(i) == 9 {
        i += 1;
    }
    let neg = at(i) == b'-';
    if at(i) == b'-' || at(i) == b'+' {
        i += 1;
    }
    // Stack frame: exponent buffer at -$18 (4 bytes), mantissa at -$14.
    let mut frame = vec![0u8; 0x18 + text.len() + 2];
    let mbuf = 4usize;
    let mut a5 = mbuf;
    let mut dots = 0;
    let mut fd: i16 = 0;
    while at(i) != 0 && at(i) != b'e' && at(i) != b'E' {
        if at(i) == b'.' {
            dots += 1;
        } else {
            frame[a5] = at(i);
            a5 += 1;
            if dots != 0 {
                fd = fd.wrapping_add(1);
            }
        }
        i += 1;
    }
    frame[a5] = 0;
    let mut eneg = false;
    let mut a4 = 0usize;
    if at(i) == b'e' || at(i) == b'E' {
        i += 1;
        eneg = at(i) == b'-';
        if at(i) == b'-' || at(i) == b'+' {
            i += 1;
        }
        while at(i) != 0 {
            frame[a4] = at(i);
            a4 += 1;
            i += 1;
        }
    }
    frame[a4] = 0;
    // L280A8: digits of the mantissa (`v = v * 10 + digit` in FFP; exact,
    // so done with integers, while the value stays below 2^24).
    let mut v = 0u32;
    let mut vi = 0u32;
    let mut exact = true;
    let mut p = mbuf;
    while (frame[p] as i8) >= b'0' as i8 && frame[p] <= b'9' {
        let d = (frame[p] - b'0') as u32;
        p += 1;
        if exact {
            let n = vi * 10 + d;
            if n < EXACT_INT {
                vi = n;
                continue;
            }
            exact = false;
            v = ffp_from_long(vi as i32);
        }
        v = ffp_mul(v, TEN);
        v = ffp_add(ffp_from_long(d as i32), v);
    }
    if exact {
        v = ffp_from_long(vi as i32);
    }
    // L28654: exponent.
    let mut q = 0;
    let mut eneg2 = false;
    if frame[q] == b'+' {
        q += 1;
    } else if frame[q] == b'-' {
        q += 1;
        eneg2 = true;
    }
    let mut e: i16 = 0;
    while (frame[q] as i8) >= b'0' as i8 && frame[q] <= b'9' {
        e = e.wrapping_mul(10).wrapping_add(frame[q] as i16).wrapping_sub(0x30);
        q += 1;
    }
    if eneg2 {
        e = e.wrapping_neg();
    }
    let pw = if eneg { e.wrapping_neg() } else { e }.wrapping_sub(fd);
    // L28040: 10^pw (repeated multiplications or divisions of 1.0).
    let scale = if pw < 0 { pow10(-(pw as i32) as u32, false) } else { pow10(pw as u32, true) };
    let r = renormalise(ffp_mul(scale, v));
    Ffp(if neg { r | 0x80 } else { r })
}

/// `L28116`: rebuilds an FFP value through an integer mantissa. The
/// halvings / doublings and the final conversion are exact, so a normalised
/// value (mantissa bit 31 set, exponent not 0) comes back unchanged.
fn renormalise(x: u32) -> u32 {
    if x & 0x8000_0000 != 0 && x & 0x7F != 0 {
        return x;
    }
    let mut x = x;
    if ffp_cmp(x, 0) == Ordering::Equal {
        return 0;
    }
    let mut neg = false;
    if ffp_cmp(x, 0) == Ordering::Less {
        x = ffp_neg(x);
        neg = true;
    }
    let mut e: i16 = 0;
    while ffp_cmp(x, ONE) != Ordering::Less {
        e += 1;
        x = ffp_div(x, TWO);
    }
    while ffp_cmp(x, HALF) == Ordering::Less {
        e -= 1;
        x = ffp_mul(x, TWO);
    }
    x = ffp_mul(x, TWO24);
    let mut n = (ffp_to_long(x) as u32) << 8;
    n |= (e.wrapping_add(0x40) & 0x7F) as u32;
    if neg {
        n |= 0x80;
    }
    n
}

// ---------------------------------------------------------------------------
// FFP -> text
// ---------------------------------------------------------------------------

/// Print/Str$ precision state (`FixFlg` / `ExpFlg`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fix {
    /// `FixFlg=-1, ExpFlg=0` (default, `Fix 16`).
    Free,
    /// `Fix n`, 0 <= n < 16.
    Decimals(u8),
    /// `Fix -n`, 0 < n < 16: exponential with n decimals.
    Exponent(u8),
    /// `Fix -n` with n >= 16: exponential, proportional digits
    /// (`FixFlg=-1, ExpFlg=1`).
    ExponentFree,
}

impl Fix {
    /// Model of `InFix` (`+Lib.s:1929`).
    pub fn from_fix_arg(n: i32) -> Fix {
        let exp = n < 0;
        let n = n.wrapping_neg_if(exp) as u32;
        match (n < 16, exp) {
            (true, false) => Fix::Decimals(n as u8),
            (true, true) => Fix::Exponent(n as u8),
            (false, false) => Fix::Free,
            (false, true) => Fix::ExponentFree,
        }
    }

    /// `FixFlg` word.
    pub fn fix_flg(self) -> i16 {
        match self {
            Fix::Free | Fix::ExponentFree => -1,
            Fix::Decimals(n) | Fix::Exponent(n) => n as i16,
        }
    }

    /// `ExpFlg` word.
    pub fn exp_flg(self) -> i16 {
        match self {
            Fix::Exponent(_) | Fix::ExponentFree => 1,
            _ => 0,
        }
    }
}

trait NegIf {
    fn wrapping_neg_if(self, c: bool) -> Self;
}
impl NegIf for i32 {
    fn wrapping_neg_if(self, c: bool) -> i32 {
        if c { self.wrapping_neg() } else { self }
    }
}

/// `ffp2a`'s first part: the sign, and the value scaled to `[1, 10)` with
/// its decimal exponent (repeated multiplications / divisions by 10). The
/// formatting routines convert the same value up to twice: this part is
/// done once.
#[derive(Clone, Copy)]
struct Norm {
    neg: bool,
    x: u32,
    e: i16,
}

fn ffp2a_norm(x: u32) -> Norm {
    let mut x = x;
    let mut e: i16 = 0;
    let neg = ffp_cmp(x, 0) == Ordering::Less;
    if neg {
        x = ffp_neg(x);
    }
    if ffp_cmp(x, 0) == Ordering::Greater {
        while ffp_cmp(x, ONE) == Ordering::Less {
            x = mul_ten(x);
            e -= 1;
        }
    }
    while ffp_cmp(x, TEN) != Ordering::Less {
        x = div_ten(x);
        e += 1;
    }
    Norm { neg, x, e }
}

/// Number of digits of `ffp2a` and its rounding: `x` plus half a unit of
/// the last digit (`10^-(ndig-1) / 2`, built by divisions from 1.0: a
/// table), and the exponent after a carry to 10.
fn ffp2a_round(n: &Norm, prec: i16) -> (u32, i16, i16) {
    let ndig: i16 = if prec <= 0 {
        1
    } else if prec > 22 {
        23
    } else {
        prec + 1
    };
    let ndig = ndig.wrapping_add(n.e);
    let k = if ndig > 1 { ndig as usize - 1 } else { 0 };
    let half = if k < POW10_LEN { HALF_DOWN[k] } else { ffp_div(pow10(k as u32, false), TWO) };
    let mut x = ffp_add(n.x, half);
    let mut e = n.e;
    if ffp_cmp(x, TEN) != Ordering::Less {
        x = ONE;
        e += 1;
    }
    (x, e, ndig)
}

/// `ffp2a` (`+Lib.s:26226`): C style ftoa computed in FFP.
fn ffp2a(x: u32, prec: i16) -> Vec<u8> {
    ffp2a_n(&ffp2a_norm(x), prec)
}

fn ffp2a_n(n: &Norm, prec: i16) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    if n.neg {
        out.push(b'-');
    }
    let (mut x, mut e, ndig) = ffp2a_round(n, prec);
    if e < 0 {
        out.extend_from_slice(b"0.");
        if ndig < 0 {
            e = e.wrapping_sub(ndig);
        }
        let mut i: i16 = -1;
        while i > e {
            out.push(b'0');
            i -= 1;
        }
    }
    let mut i: i16 = 0;
    while i < ndig {
        let d = ffp_to_long(x) as i16;
        out.push((d as u8).wrapping_add(b'0'));
        if i == e {
            out.push(b'.');
        }
        x = mul_ten(sub_int_part(x, d as i32));
        i += 1;
    }
    out
}

/// `ffp_mul(x, TEN)` for a positive normalised `x` with exponent byte in
/// `[2, 0x7B]` (else the general routine): the mantissa product is
/// `M * 0xA000 / 256 = M * 160` exactly (the low word of 10 is 0), then
/// the routine's rounding and normalisation.
fn mul_ten(x: u32) -> u32 {
    let eb = x & 0xFF;
    if !(2..=0x7B).contains(&eb) || x & 0x8000_0000 == 0 {
        return ffp_mul(x, TEN);
    }
    let mut d7 = (x >> 8) * 160;
    let mut eb = eb + 4;
    if d7 & 0x8000_0000 != 0 {
        d7 += 0x80;
        return (d7 & !0xFF) | eb;
    }
    eb -= 1;
    d7 += 0x40;
    let c = d7 & 0x8000_0000 != 0;
    d7 = d7.wrapping_add(d7);
    if c {
        d7 = (d7 >> 1) | 0x8000_0000;
        eb += 1;
    }
    (d7 & !0xFF) | eb
}

/// `ffp_div(x, TEN)` with the divisor folded in: 10 has a zero low
/// mantissa word, so the correction step disappears and the two DIVU.W
/// give `m * 2^16 / $A000` exactly (falls back where a DIVU.W would
/// overflow or the exponent leaves the range).
fn div_ten(x: u32) -> u32 {
    if x == 0 {
        return 0;
    }
    let a = ((lb(x) << 1) ^ 0x80) as i8;
    let (diff, ov) = a.overflowing_sub(8);
    if ov {
        return ffp_div(x, TEN);
    }
    let mut d4b = diff as u8;
    let mut m = x & !0xFF;
    if ((m >> 16).wrapping_sub(0xA000) & 0x8000) == 0 {
        if (d4b as i8).checked_add(2).is_none() {
            return ffp_div(x, TEN);
        }
        d4b = d4b.wrapping_add(2);
        m >>= 1;
    }
    let q1 = m / 0xA000;
    if q1 > 0xFFFF {
        return ffp_div(x, TEN);
    }
    let q2 = ((m % 0xA000) << 16) / 0xA000;
    let mut eb = ((lb(x) ^ 0x44) & 0x80) | ((d4b ^ 0x80) >> 1);
    let mut d5 = (q1 << 16) | q2;
    if q1 & 0x8000 == 0 {
        d5 = d5.wrapping_add(d5);
        eb = eb.wrapping_sub(1);
    }
    d5 = d5.wrapping_add(0x80);
    if eb == 0 {
        return 0;
    }
    setb(d5, eb)
}

/// `x - d` (`ffp_sub(x, ffp_from_long(d))`) with `d` the integer part of
/// `x`. For a positive normalised `x` in `[1, 16)` the difference is exact
/// (the fraction bits of the mantissa), so it is computed directly.
fn sub_int_part(x: u32, d: i32) -> u32 {
    let eb = x & 0xFF;
    if !(0x41..=0x44).contains(&eb) || x & 0x8000_0000 == 0 {
        return ffp_sub(x, ffp_from_long(d));
    }
    let int_bits = eb - 0x40;
    let frac = (x >> 8) & ((1 << (24 - int_bits)) - 1);
    if frac == 0 {
        return 0;
    }
    let shift = frac.leading_zeros() - 8;
    ((frac << shift) << 8) | (eb - shift)
}

/// What `F2a` reads of `ffp2a(x, prec)` without making the text, for a
/// normalised non-zero `x`: `Ok(n)` when it starts with a digit other than
/// 0 (`n`: integer digits + 1, the '.' following them), `Err(z)` when it
/// starts with "0." (`z`: zeros after "0." + 1, a digit other than 0 or the
/// end following them). The first digit of a non-zero value is 1..9.
fn ffp2a_shape(n: &Norm, prec: i16) -> Result<i16, i16> {
    let (_, e, ndig) = ffp2a_round(n, prec);
    if e >= 0 {
        // ndig > e: the '.' follows digit e.
        return Ok(e + 2);
    }
    let e = if ndig < 0 { e.wrapping_sub(ndig) } else { e };
    Err((-1 - e).max(0) + 1)
}

/// `F2a` (`+Lib.s:25976`): `fix` = FixFlg word, `exp` = ExpFlg word.
fn f2a(x: u32, fix: i16, exp: i16) -> Vec<u8> {
    if exp == 0 && fix >= 0 {
        let f = if fix >= 8 { 7 } else { fix };
        let mut s = ffp2a(x, f);
        if f == 0 && s.last() == Some(&b'.') {
            s.pop();
        }
        return s;
    }
    let eb = lb(x) & 0x7F;
    let prec = if eb >= 0x41 {
        7
    } else if eb >= 0x31 {
        10
    } else {
        22
    };
    let nrm = ffp2a_norm(x);
    let shape =
        if x & 0x8000_0000 != 0 && eb != 0 { ffp2a_shape(&nrm, prec) } else { text_shape(&ffp2a_n(&nrm, prec)) };
    match shape {
        Ok(n) => {
            if exp != 0 || n >= 8 {
                return ex_fix(&nrm, n, fix);
            }
            let d = (7 - n).min(5);
            clean(&nrm, d)
        }
        Err(z) => {
            let mut z = z;
            let mut zero = false;
            if z >= 22 {
                z = 6;
                zero = true;
            } else if z >= 4 {
                return ex_vir(&nrm, z, fix, false);
            }
            if exp != 0 {
                return ex_vir(&nrm, z, fix, zero);
            }
            clean(&nrm, z + 6)
        }
    }
}

/// `F2a` reading the text of `ffp2a` (see `ffp2a_shape`).
fn text_shape(s: &[u8]) -> Result<i16, i16> {
    let a1 = usize::from(s.first() == Some(&b'-'));
    let at = |i: usize| s.get(i).copied().unwrap_or(0);
    if at(a1) != b'0' {
        // Number >= 1: count the integer digits (+1).
        let mut p = a1;
        loop {
            let c = at(p);
            p += 1;
            if c == 0 || c == b'.' {
                break;
            }
        }
        return Ok((p - a1) as i16);
    }
    // Number < 1: count the zeros after "0." (+1).
    let start = a1 + 2;
    let mut p = start;
    loop {
        let c = at(p);
        p += 1;
        if c == 0 || c != b'0' {
            break;
        }
    }
    Err((p - start) as i16)
}

/// `Clean`: ffp2a then strip trailing zeros (and a bare point).
fn clean(x: &Norm, dec: i16) -> Vec<u8> {
    let mut s = ffp2a_n(x, dec);
    if let Some(dot) = s.iter().position(|&c| c == b'.') {
        let mut end = dot;
        for (k, &c) in s.iter().enumerate().skip(dot + 1) {
            if c != b'0' {
                end = k + 1;
            }
        }
        s.truncate(end);
    }
    s
}

fn push_exp(out: &mut Vec<u8>, sign: u8, mut d2: i16) {
    out.push(b'E');
    out.push(sign);
    out.push(b'0');
    while (d2 as u8) >= 10 {
        *out.last_mut().unwrap() += 1;
        d2 -= 10;
    }
    out.push((d2 as u8).wrapping_add(b'0'));
}

/// `ExFix1`: exponential form for numbers >= 1.
fn ex_fix(x: &Norm, n: i16, fix: i16) -> Vec<u8> {
    let d2 = n - 2;
    let n2 = n.min(7);
    let s = ffp2a_n(x, 9 - n2);
    let mut out = Vec::new();
    let mut p = 0;
    if s.first() == Some(&b'-') {
        out.push(b'-');
        p = 1;
    }
    out.push(s.get(p).copied().unwrap_or(0));
    p += 1;
    out.push(b'.');
    let dot = out.len() - 1;
    let mut d1: i16 = if (0..5).contains(&fix) { fix } else { 5 };
    loop {
        let c = s.get(p).copied().unwrap_or(0);
        p += 1;
        if c == 0 {
            break;
        }
        if c == b'.' {
            continue;
        }
        out.push(c);
        d1 = d1.wrapping_sub(1);
        if d1 == 0 {
            break;
        }
    }
    if fix < 0 {
        let mut end = dot;
        for (k, &c) in out.iter().enumerate().skip(dot + 1) {
            if c != b'0' {
                end = k + 1;
            }
        }
        out.truncate(end);
    }
    push_exp(&mut out, b'+', d2);
    out
}

/// `ExVir1`: exponential form for numbers < 1.
fn ex_vir(x: &Norm, z: i16, fix: i16, zero: bool) -> Vec<u8> {
    let (s, d2) = if zero { (b"0.0000000".to_vec(), 0) } else { (ffp2a_n(x, z + 6), z) };
    let mut out = Vec::new();
    let mut p = 0;
    if s.first() == Some(&b'-') {
        out.push(b'-');
        p = 1;
    }
    let after_point = p + 2;
    let first;
    loop {
        let c = s.get(p).copied().unwrap_or(0);
        p += 1;
        if c == 0 {
            p = after_point;
            first = b'0';
            break;
        }
        if c == b'.' || c == b'0' {
            continue;
        }
        first = c;
        break;
    }
    let mut d1: i16 = if (0..6).contains(&fix) { fix } else { 6 };
    out.push(first);
    out.push(b'.');
    let mut keep = out.len() - 1;
    while d1 != 0 {
        let c = s.get(p).copied().unwrap_or(0);
        p += 1;
        if c != 0 {
            out.push(c);
            if c != b'0' {
                keep = out.len();
            }
        }
        d1 -= 1;
    }
    if fix < 0 {
        out.truncate(keep);
    }
    if zero {
        push_exp(&mut out, b'+', 0);
    } else {
        push_exp(&mut out, b'-', d2);
    }
    out
}

/// `FloatToAsc` post-processing (with `d4` bit 31 clear: leading space).
fn float_to_asc(x: u32, fix: i16, exp: i16) -> Vec<u8> {
    float_to_asc_flags(x, fix, exp, true)
}

fn float_to_asc_flags(x: u32, fix: i16, exp: i16, space: bool) -> Vec<u8> {
    let s = f2a(x, fix, exp);
    let mut out = Vec::new();
    if space && s.first() != Some(&b'-') {
        out.push(b' ');
    }
    if fix >= 0 {
        out.extend_from_slice(&s);
        return out;
    }
    let Some(dot) = s.iter().position(|&c| c == b'.') else {
        out.extend_from_slice(&s);
        return out;
    };
    out.extend_from_slice(&s[..dot]);
    let mut end = dot + 1;
    let mut last = dot + 1;
    while end < s.len() && s[end] != b'E' {
        if s[end] != b'0' {
            last = end + 1;
        }
        end += 1;
    }
    if last != dot + 1 {
        out.push(b'.');
        out.extend_from_slice(&s[dot + 1..last]);
    }
    if s.get(end) == Some(&b'E') {
        out.push(b' ');
    }
    out.extend_from_slice(&s[end..]);
    out
}

fn latin(v: Vec<u8>) -> String {
    // Number texts are ASCII: no copy then (Latin-1 otherwise).
    if v.is_ascii() {
        return String::from_utf8(v).unwrap_or_default();
    }
    v.into_iter().map(|c| c as char).collect()
}

/// Text of a single precision constant as the editor lists it (`DtkC6`,
/// `+Edit.s:15051`): `FloatToAsc` with no sign space and proportional digits,
/// then ".0" appended unless the text holds a '.' or an 'E'.
pub fn format_ffp_listing(v: Ffp) -> String {
    let mut s = float_to_asc_flags(v.0, -1, 0, false);
    if !s.iter().any(|&c| c == b'.' || c == b'E') {
        s.extend_from_slice(b".0");
    }
    latin(s)
}

/// Text of a double precision constant as the editor lists it (`DtkC7`):
/// `DoubleToAsc` with 15 digits in mode 2, then ".0" appended unless the
/// text holds a '.' or an 'E' (the exponent is a lower case 'e', so
/// `1e+020` is listed as `1e+020.0`).
pub fn format_double_listing(v: f64) -> String {
    let mut s = dtoa(v, 15, 2);
    if !s.iter().any(|&c| c == b'.' || c == b'E') {
        s.extend_from_slice(b".0");
    }
    latin(s)
}

/// Formats a single precision float as `Print` / `Str$` do (`Float2Ascii`
/// -> `FloatToAsc` -> `F2a` -> `ffp2a`), including the leading space for
/// numbers that do not start with `-`.
pub fn format_ffp(v: Ffp, fix: Fix) -> String {
    latin(float_to_asc(v.0, fix.fix_flg(), fix.exp_flg()))
}

/// `Str$` / `Print` of an integer (`LongToAsc` signed, proportional).
pub fn format_int(v: i32) -> String {
    if v < 0 { format!("-{}", (v as i64).unsigned_abs()) } else { format!(" {v}") }
}

// ---------------------------------------------------------------------------
// Double -> text (Float2AsciiD / Dtoa)
// ---------------------------------------------------------------------------

/// `DDebut` table: 10.0, 1.0, then 0.5, 0.05, ... 5e-16 as stored.
pub(crate) const DTAB: [u64; 18] = [
    0x4024_0000_0000_0000,
    0x3FF0_0000_0000_0000,
    0x3FE0_0000_0000_0000,
    0x3FA9_9999_9999_999A,
    0x3F74_7AE1_47AE_147B,
    0x3F40_624D_D2F1_A9FC,
    0x3F0A_36E2_EB1C_432D,
    0x3ED4_F8B5_88E3_68F1,
    0x3EA0_C6F7_A0B5_ED8E,
    0x3E6A_D7F2_9ABC_AF49,
    0x3E35_798E_E230_8C3A,
    0x3E01_2E0B_E826_D695,
    0x3DCB_7CDF_D9D7_BDBB,
    0x3D95_FD7F_E179_6496,
    0x3D61_9799_812D_EA12,
    0x3D2C_25C2_6849_7682,
    0x3CF6_849B_86A1_2B9C,
    0x3CC2_03AF_9EE7_5616,
];

/// `Dtoa` (`+Lib.s:27080`) with the '#' flag forced on. The original
/// computes with the double routines of its C runtime (`softdouble`; the
/// instruction by instruction transcription is `softdouble::reference::
/// dtoa`), which are not correctly rounded in general; on the values that
/// reach them here they give the IEEE results, so `f64` arithmetic is
/// bit-exact (tests `softdouble::dtoa_operations_are_ieee` and
/// `dtoa_matches_the_transcription`):
///
/// * x * 10 (normal x < 1): 10 has one mantissa word and x eleven zero low
///   bits, so the product is exact before its single rounding;
/// * x / 10 (x >= 10): the truncated quotient n * 2^14 / 5 is never a half
///   with a non zero remainder;
/// * x - trunc(x), the truncation and the comparisons: exact;
/// * x + 5 * 10^-k (1 <= x < 10): the addition keeps a sticky bit, and the
///   one case where it loses it (a carry) needs bits of the aligned
///   constant that are never all zero for these constants and exponents
///   (the other tie cases cannot occur);
/// * zero and unnormalised x (zero exponent field): treated as 0, as the
///   original's routines do.
fn dtoa(x: f64, ndig: i32, mode: i32) -> Vec<u8> {
    let mut out = Vec::new();
    let bits = x.to_bits();
    if (bits >> 52) & 0x7FF == 0x7FF {
        let c = if bits >> 63 != 0 { b'-' } else { b'+' };
        for _ in 0..ndig.max(0) {
            out.push(c);
        }
        return out;
    }
    let ten = f64::from_bits(DTAB[0]);
    let one = f64::from_bits(DTAB[1]);
    let sgn = |v: f64| -> i32 {
        let b = v.to_bits();
        if (b >> 52) & 0x7FF == 0 {
            0
        } else if b >> 63 != 0 {
            -1
        } else {
            1
        }
    };
    let mut x = x;
    let mut d4: i32 = 0;
    if sgn(x) < 0 {
        x = -x;
        out.push(b'-');
    }
    if sgn(x) > 0 {
        while x < one {
            x *= ten;
            d4 -= 1;
        }
        while x >= ten {
            x /= ten;
            d4 += 1;
        }
    } else {
        x = 0.0;
    }
    let mut d6 = ndig;
    let mut d7 = mode & 3;
    let mut d5;
    if d7 == 2 {
        if d6 == 0 {
            d6 = 1;
        }
        if d4 < -4 || d4 >= d6 {
            d7 = -1;
        }
        d5 = d6;
    } else if d7 == 1 {
        d5 = d6 + d4 + 1;
    } else {
        d5 = d6 + 1;
    }
    if d5 > 0 {
        let k = d5.min(16) as usize;
        x += f64::from_bits(DTAB[k + 1]);
        if x >= ten {
            x = one;
            d4 += 1;
            if d7 > 0 {
                d5 += 1;
            }
        }
    }
    let mut a6: i32;
    if d7 > 0 {
        if d4 < 0 {
            out.extend_from_slice(b"0.");
            let z = if d5 > 0 { -d4 - 1 } else { d6 };
            out.resize(out.len() + z.max(0) as usize, b'0');
            a6 = 0;
        } else {
            a6 = d4 + 1;
        }
    } else {
        a6 = 1;
    }
    if d5 > 0 {
        let mut a3 = 0;
        loop {
            if a3 < 16 {
                let d = x.trunc() as i32;
                out.push((d as u8).wrapping_add(b'0'));
                x = (x - d as f64) * ten;
            } else {
                out.push(b'0');
            }
            d5 -= 1;
            if d5 == 0 {
                break;
            }
            if a6 != 0 {
                a6 -= 1;
                if a6 == 0 {
                    out.push(b'.');
                }
            }
            a3 += 1;
        }
    }
    if a6 != 0 {
        out.push(b'.');
    }
    if d7 <= 0 {
        out.push(b'e');
        if d4 < 0 {
            d4 = -d4;
            out.push(b'-');
        } else {
            out.push(b'+');
        }
        out.push((d4 / 100) as u8 + b'0');
        let r = d4 % 100;
        out.push((r / 10) as u8 + b'0');
        out.push((r % 10) as u8 + b'0');
    }
    if mode & 0xFF == 2 {
        // Remove the zeros after the point (and a bare point).
        let mut point = None;
        let mut last = None;
        let mut end = out.len();
        for (k, &c) in out.iter().enumerate() {
            if c == b'e' || c == b'E' {
                end = k;
                break;
            }
            if c == b'.' {
                point = Some(k + 1);
            } else if c != b'0' && point.is_some() {
                last = Some(k + 1);
            }
        }
        if let Some(pt) = point {
            let to = last.unwrap_or(pt - 1);
            out.drain(to..end);
        }
    }
    out
}

/// Formats a double precision float as `Print` / `Str$` do in double
/// precision mode (`Float2AsciiD`, `+ILib.s:7684`): `%g` style with 15
/// significant digits by default, `Fix n` = n significant digits, `Fix -n` =
/// exponential with n decimals, exponents as `e+020`.
pub fn format_double(v: f64, fix: Fix) -> String {
    let (mut mode, mut ndig) = (2, 15);
    if fix.fix_flg() >= 0 {
        ndig = fix.fix_flg() as i32;
        if fix.exp_flg() != 0 {
            mode = 0;
        }
    }
    let mut out = Vec::new();
    if v.to_bits() >> 63 == 0 {
        out.push(b' ');
    }
    out.extend(dtoa(v, ndig, mode));
    latin(out)
}

#[cfg(test)]
#[path = "ffp_reference.rs"]
mod reference;

/// The shortcuts of the conversions against the original algorithms
/// (`ffp_reference.rs`).
#[cfg(test)]
mod shortcut_tests {
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

    fn fixes() -> Vec<Fix> {
        let mut v = vec![Fix::Free, Fix::ExponentFree];
        v.extend((0..16).map(Fix::Decimals));
        v.extend((1..16).map(Fix::Exponent));
        v
    }

    fn check_format(x: u32, fixes: &[Fix]) {
        for &f in fixes {
            let want = reference::float_to_asc(x, f.fix_flg(), f.exp_flg());
            assert_eq!(float_to_asc(x, f.fix_flg(), f.exp_flg()), want, "x={x:08x} fix={f:?}");
        }
        let want = reference::float_to_asc_flags(x, -1, 0, false);
        assert_eq!(float_to_asc_flags(x, -1, 0, false), want, "listing x={x:08x}");
    }

    /// Debug builds run fewer iterations.
    fn count(n: usize) -> usize {
        if cfg!(debug_assertions) { n / 8 } else { n }
    }

    #[test]
    fn tables_are_the_loops() {
        let (mut up, mut down) = (ONE, ONE);
        for k in 0..300u32 {
            assert_eq!(pow10(k, true), up, "10^{k}");
            assert_eq!(pow10(k, false), down, "10^-{k}");
            if (k as usize) < POW10_LEN {
                assert_eq!(HALF_DOWN[k as usize], ffp_div(down, TWO));
            }
            up = ffp_mul(up, TEN);
            down = ffp_div(down, TEN);
        }
    }

    #[test]
    fn digit_accumulation_below_2_24_is_exact() {
        // Every step of the integer shortcut of `a2ffp`.
        for v in 0..EXACT_INT / 10 + 1 {
            let fv = ffp_mul(ffp_from_long(v as i32), TEN);
            for d in 0..10 {
                let n = v * 10 + d;
                if n < EXACT_INT {
                    assert_eq!(ffp_add(ffp_from_long(d as i32), fv), ffp_from_long(n as i32), "{v}*10+{d}");
                }
            }
        }
    }

    #[test]
    fn renormalise_of_normalised_values() {
        let mut r = Rng(0x1234_5678_9ABC_DEF1);
        // Every mantissa at a few exponents, both signs.
        let step = if cfg!(debug_assertions) { 7 } else { 1 };
        for exp in [0x01, 0x30, 0x40, 0x41, 0x58, 0x7F] {
            for m in (0x80_0000u32..0x100_0000).step_by(step) {
                for sign in [0, 0x80] {
                    let x = (m << 8) | sign | exp;
                    assert_eq!(renormalise(x), reference::renormalise(x), "{x:08x}");
                }
            }
        }
        for _ in 0..count(2_000_000) {
            let x = (r.next() as u32) | 0x8000_0000;
            if x & 0x7F != 0 {
                assert_eq!(renormalise(x), reference::renormalise(x), "{x:08x}");
            }
        }
    }

    #[test]
    fn multiplication_by_ten() {
        let step = if cfg!(debug_assertions) { 3 } else { 1 };
        // Every mantissa at the exponents of the digit loop and the ends.
        for eb in [0x01, 0x02, 0x20, 0x3C, 0x3D, 0x3E, 0x3F, 0x40, 0x41, 0x42, 0x43, 0x44, 0x7A, 0x7B, 0x7C, 0x7F] {
            for m in (0x80_0000u32..0x100_0000).step_by(step) {
                for sign in [0, 0x80] {
                    let x = (m << 8) | sign | eb;
                    assert_eq!(mul_ten(x), ffp_mul(x, TEN), "{x:08x}");
                }
            }
        }
        for eb in 0..0x80u32 {
            for m in [0x80_0000u32, 0x80_0001, 0xCC_CCCC, 0xCC_CCCD, 0xE6_6666, 0xFF_FFFF] {
                for sign in [0, 0x80] {
                    let x = (m << 8) | sign | eb;
                    assert_eq!(mul_ten(x), ffp_mul(x, TEN), "{x:08x}");
                }
            }
        }
        let mut r = Rng(0x1357_2468_FEDC_BA98);
        for _ in 0..count(2_000_000) {
            let x = r.next() as u32;
            assert_eq!(mul_ten(x), ffp_mul(x, TEN), "{x:08x}");
        }
    }

    #[test]
    fn division_by_ten() {
        let step = if cfg!(debug_assertions) { 3 } else { 1 };
        for eb in [0x00, 0x01, 0x03, 0x04, 0x05, 0x40, 0x41, 0x44, 0x45, 0x48, 0x5F, 0x7E, 0x7F] {
            for m in (0x80_0000u32..0x100_0000).step_by(step) {
                for sign in [0, 0x80] {
                    let x = (m << 8) | sign | eb;
                    assert_eq!(div_ten(x), ffp_div(x, TEN), "{x:08x}");
                }
            }
        }
        for eb in 0..0x100u32 {
            for m in [0x80_0000u32, 0x80_0001, 0x9F_FFFF, 0xA0_0000, 0xA0_0001, 0xCC_CCCD, 0xFF_FFFF, 0x1, 0x7F_FFFF] {
                let x = (m << 8) | eb;
                assert_eq!(div_ten(x), ffp_div(x, TEN), "{x:08x}");
            }
        }
        let mut r = Rng(0x9999_1111_2222_3333);
        for _ in 0..count(4_000_000) {
            let x = r.next() as u32;
            assert_eq!(div_ten(x), ffp_div(x, TEN), "{x:08x}");
        }
    }

    fn random_double(r: &mut Rng) -> u64 {
        let v = r.next();
        match r.below(6) {
            0 => v,
            1 => (v & 0x800F_FFFF_FFFF_FFFF) | ((r.below(3) + 0x7FD * r.below(2)) << 52),
            2 => (r.next() as i32 as f64 / [1.0, 7.0, 100.0, 3.0][r.below(4) as usize]).to_bits(),
            3 => crate::softdouble::asc_to_double(format!("{}.{}", r.below(100000), r.below(1000)).as_bytes()),
            _ => (v & 0x800F_FFFF_FFFF_FFFF) | ((0x3FF - 70 + r.below(140)) << 52),
        }
    }

    #[test]
    fn dtoa_matches_the_transcription() {
        let mut r = Rng(0xD70A_1234_5678_9ABC);
        let mut xs: Vec<u64> = vec![0, 1 << 63, 0x7FF0_0000_0000_0000, 0xFFF8_0000_0000_0001, 0x0000_0000_0000_0001];
        xs.extend(DTAB);
        for k in 0..400 {
            xs.push(crate::softdouble::pow10(k));
            xs.push(crate::softdouble::pow10(k) - 1);
            xs.push(crate::softdouble::pow10(k) + 1);
        }
        for p in [2.0f64, 4.0, 8.0, 10.0, 1.0] {
            for d in 1..2000u64 {
                xs.push(p.to_bits() - d);
                xs.push(p.to_bits() - (d << 20));
            }
        }
        for _ in 0..count(200_000) {
            xs.push(random_double(&mut r));
        }
        for (i, &x) in xs.iter().enumerate() {
            // Print / Str$ (mode 2, 15 digits or Fix n; mode 0 for Fix -n).
            for (ndig, mode) in [(15, 2), ((i % 17) as i32, 2), ((i % 17) as i32, 0), ((i % 23) as i32, 1)] {
                let want = crate::softdouble::reference::dtoa(x, ndig, mode);
                assert_eq!(dtoa(f64::from_bits(x), ndig, mode), want, "{x:016X} ndig={ndig} mode={mode}");
            }
        }
    }

    /// The original's `Dtoa` (the transcription, with its double routines)
    /// against `dtoa` on more values: `SURVEY_N=... cargo test --release -p
    /// amos-core survey_dtoa -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn survey_dtoa_vs_transcription() {
        let mut r = Rng(0x5EED_D7A0_0000_0001);
        type Gen = Box<dyn Fn(&mut Rng) -> f64>;
        let cats: Vec<(&str, Gen)> = vec![
            ("integers", Box::new(|r| (r.next() >> r.below(64)) as i64 as f64)),
            ("I/7 (I < 10^6)", Box::new(|r| r.below(1_000_000) as f64 / 7.0)),
            (
                "short decimals (Val)",
                Box::new(|r| {
                    let t = format!("{}.{}", r.below(100000), r.below(1000));
                    f64::from_bits(crate::softdouble::asc_to_double(t.as_bytes()))
                }),
            ),
            (
                "random bits (normal range)",
                Box::new(|r| f64::from_bits((r.next() & 0x800F_FFFF_FFFF_FFFF) | ((0x3FF - 200 + r.below(400)) << 52))),
            ),
            (
                "just below 2, 4, 8, 10 (rounding carries)",
                Box::new(|r| {
                    f64::from_bits([2.0f64, 4.0, 8.0, 10.0][r.below(4) as usize].to_bits() - 1 - r.below(1 << 40))
                }),
            ),
            ("any bits", Box::new(|r| f64::from_bits(r.next()))),
        ];
        let n: usize = std::env::var("SURVEY_N").ok().and_then(|v| v.parse().ok()).unwrap_or(300_000);
        for (name, g) in &cats {
            let mut diff = 0;
            for i in 0..n {
                let x = g(&mut r);
                let (nd, mode) = [(15, 2), (4, 2), (10, 0), ((i % 17) as i32, 1)][i % 4];
                if dtoa(x, nd, mode) != crate::softdouble::reference::dtoa(x.to_bits(), nd, mode) {
                    diff += 1;
                }
            }
            println!("{name}: {diff} of {n} differ");
        }
    }

    #[test]
    fn integer_part_subtraction() {
        // Every positive normalised value in [1, 16), and others.
        let step = if cfg!(debug_assertions) { 5 } else { 1 };
        for eb in 0x41..=0x44u32 {
            for m in (0x80_0000u32..0x100_0000).step_by(step) {
                let x = (m << 8) | eb;
                let d = ffp_to_long(x);
                assert_eq!(sub_int_part(x, d), ffp_sub(x, ffp_from_long(d)), "{x:08x}");
            }
        }
        let mut r = Rng(0x2468_ACE0_1357_9BDF);
        for _ in 0..count(1_000_000) {
            let x = random_ffp(&mut r);
            let d = ffp_to_long(x) as i16 as i32;
            assert_eq!(sub_int_part(x, d), ffp_sub(x, ffp_from_long(d)), "{x:08x}");
        }
    }

    fn random_ffp(r: &mut Rng) -> u32 {
        let v = r.next();
        let mant = 0x80_0000 | ((v >> 16) as u32 & 0x7F_FFFF);
        let exp = match (v >> 40) % 4 {
            0 => (v >> 48) as u32 & 0x7F,
            _ => 40 + (v >> 48) as u32 % 50,
        };
        (mant << 8) | (((v >> 60) as u32 & 1) << 7) | exp
    }

    #[test]
    fn formatting_matches_the_original() {
        let fixes = fixes();
        // Special and non-normalised values.
        for x in [0, 0x80, 0x8000_0041, 0x8000_00C1, 0xFFFF_FF7F, 0xFFFF_FFFF, 0x8000_0001, 0x8000_0081] {
            check_format(x, &fixes);
        }
        // Every exponent with mantissa edges, and both signs.
        for exp in 0..0x80u32 {
            for m in
                [0x80_0000u32, 0x80_0001, 0x9F_FFFF, 0xA0_0000, 0xC8_0000, 0xCC_CCCD, 0xF9_FFFF, 0xFF_FFFE, 0xFF_FFFF]
            {
                for sign in [0, 0x80] {
                    check_format((m << 8) | sign | exp, &fixes);
                }
            }
        }
        // Values next to powers of ten and to the rounding carries (9.99..).
        for k in 0..POW10_LEN as u32 {
            for p in [pow10(k, true), pow10(k, false)] {
                for d in -40i32..=40 {
                    let m = ((p >> 8) as i32 + d) as u32;
                    if (0x80_0000..0x100_0000).contains(&m) {
                        check_format((m << 8) | (p & 0xFF), &fixes);
                    }
                }
            }
        }
        for t in ["9.9999995", "99999.995", "0.99999995", "9999999.5", "0.000099999995", "1e-5", "1e-4", "123456789"] {
            check_format(reference::ascii_to_ffp(t.as_bytes()).0, &fixes);
        }
        // Random values.
        let mut r = Rng(0x0F0F_1234_5678_9ABC);
        for i in 0..count(400_000) {
            let x = random_ffp(&mut r);
            let f = fixes[i % fixes.len()];
            assert_eq!(
                float_to_asc(x, f.fix_flg(), f.exp_flg()),
                reference::float_to_asc(x, f.fix_flg(), f.exp_flg()),
                "x={x:08x} fix={f:?}"
            );
        }
    }

    fn check_text(t: &[u8]) {
        assert_eq!(ascii_to_ffp(t), reference::ascii_to_ffp(t), "{:?}", String::from_utf8_lossy(t));
    }

    #[test]
    fn text_conversion_matches_the_original() {
        // Around the end of the integer shortcut (2^24 = 16777216) and of
        // the tables, digit counts, exponent limits.
        for n in (16_777_100u32..16_777_300).chain(1_677_700..1_677_800).chain(167_772_000..167_772_200) {
            check_text(n.to_string().as_bytes());
            check_text(format!("{n}.5").as_bytes());
            check_text(format!("-{n}e-3").as_bytes());
            check_text(format!("0.{n}").as_bytes());
        }
        for digits in 1..40 {
            let s: String = (0..digits).map(|i| char::from(b'0' + ((i * 7 + 3) % 10) as u8)).collect();
            check_text(s.as_bytes());
            check_text(format!("9{s}").as_bytes());
            check_text(format!("{s}.{s}").as_bytes());
            check_text(format!(".{s}e7").as_bytes());
        }
        for e in -120i32..120 {
            for m in ["1", "9.99", "1.5", "123456789", "0", ".1", "16777217"] {
                check_text(format!("{m}e{e}").as_bytes());
            }
        }
        for t in ["", "-", ".", "e", "1e", "1e+", "--1", "1..2", "1e99999", "1e-99999", "1e32767", "1e-32768", "00012"]
        {
            check_text(t.as_bytes());
        }
        let mut r = Rng(0x7777_1234_ABCD_0001);
        let set = b"0123456789.e+-";
        for _ in 0..count(400_000) {
            let len = r.below(24) as usize;
            let t: Vec<u8> = match r.below(3) {
                0 => (0..len).map(|_| set[r.below(set.len() as u64) as usize]).collect(),
                1 => (0..len).map(|_| b'0' + r.below(10) as u8).collect(),
                _ => {
                    let mut t: Vec<u8> = (0..len.max(1)).map(|_| b'0' + r.below(10) as u8).collect();
                    t.insert(r.below(t.len() as u64 + 1) as usize, b'.');
                    t.extend_from_slice(format!("e{}", r.below(100) as i64 - 50).as_bytes());
                    t
                }
            };
            check_text(&t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> u32 {
        ascii_to_ffp(s.as_bytes()).0
    }

    fn p(s: &str) -> String {
        format_ffp(ascii_to_ffp(s.as_bytes()), Fix::Free)
    }

    #[test]
    fn constants() {
        assert_eq!(a("1"), 0x8000_0041);
        assert_eq!(a("10"), 0xA000_0044);
        assert_eq!(a("0.5"), 0x8000_0040);
        assert_eq!(a("15.5"), 0xF800_0044);
        assert_eq!(a("0.1"), 0xCCCC_CD3D);
        assert_eq!(a("179.35"), 0xB359_9948);
        assert_eq!(a("-1.5"), 0xC000_00C1);
        assert_eq!(a("0"), 0);
        assert_eq!(a(".5"), 0x8000_0040);
        assert_eq!(a("1e2"), 0xC800_0047);
        assert_eq!(a("1.5E-1"), ascii_to_ffp(b"0.15").0);
    }

    #[test]
    fn int_conversions() {
        assert_eq!(Ffp::from_i32(1).0, ONE);
        assert_eq!(Ffp::from_i32(-1).0, ONE | 0x80);
        assert_eq!(Ffp::from_i32(10).0, TEN);
        assert_eq!(Ffp::from_i32(0).0, 0);
        assert_eq!(Ffp::from_i32(i32::MIN).to_i32(), i32::MIN);
        assert_eq!(Ffp::from_i32(123456).to_i32(), 123456);
        assert_eq!(a("2.9").to_i32_ffp(), 2);
        assert_eq!(a("-2.9").to_i32_ffp(), -2);
        assert_eq!(a("1e15").to_i32_ffp(), i32::MAX);
        assert_eq!(a("-1e15").to_i32_ffp(), i32::MIN);
    }

    trait T {
        fn to_i32_ffp(self) -> i32;
    }
    impl T for u32 {
        fn to_i32_ffp(self) -> i32 {
            Ffp(self).to_i32()
        }
    }

    #[test]
    fn arithmetic() {
        let one = Ffp::ONE;
        let three = Ffp::from_i32(3);
        assert_eq!(one.add(one).0, TWO);
        assert_eq!(one.sub(one).0, 0);
        assert_eq!(Ffp::from_i32(5).sub(Ffp::from_i32(3)).0, TWO);
        assert_eq!(Ffp::from_i32(3).sub(Ffp::from_i32(5)).0, TWO | 0x80);
        assert_eq!(three.mul(three), Ffp::from_i32(9));
        assert_eq!(Ffp::from_i32(9).div(three), three);
        // 1/3 rounds up in the last bit.
        assert_eq!(one.div(three).0, 0xAAAA_AB3F);
        assert_eq!(Ffp::TEN.div(Ffp::from_i32(100)).0, 0xCCCC_CD3D);
        assert_eq!(one.cmp(three), Ordering::Less);
        assert_eq!(three.neg().cmp(one.neg()), Ordering::Less);
        assert_eq!(one.neg().cmp(Ffp::ZERO), Ordering::Less);
        assert_eq!(Ffp::ZERO.cmp(one.neg()), Ordering::Greater);
        assert_eq!(Ffp(a("2.5")).floor(), Ffp::from_i32(2));
        assert_eq!(Ffp(a("-2.5")).floor(), Ffp::from_i32(-3));
        assert_eq!(Ffp(a("-0.5")).floor(), Ffp::from_i32(-1));
        assert_eq!(Ffp(a("0.5")).floor(), Ffp::ZERO);
        assert_eq!(Ffp(a("-3")).floor(), Ffp::from_i32(-3));
        assert_eq!(Ffp(0xB359_9948).to_f64(), 179.34999084472656);
        assert_eq!(Ffp::from_f64(0.1).0, 0xCCCC_CC3D);
        assert_eq!(Ffp::from_f64(-15.5).0, 0xF800_00C4);
        // Motorola FFPDIV quirk: a single correction of the 16 bit quotient
        // estimate (exact value would be $ACB5F2xx).
        assert_eq!(a("3.14159"), 0xC90F_CE42);
        assert_eq!(Ffp(0xC90F_CE42).div(Ffp(0x9502_F962)).0, 0xACB6_0021);
        // Mixed signs with different exponents.
        assert_eq!(Ffp::from_i32(100).add(Ffp::from_i32(-1)), Ffp::from_i32(99));
        assert_eq!(Ffp::from_i32(-100).add(Ffp::from_i32(1)), Ffp::from_i32(-99));
        assert_eq!(Ffp::from_i32(1).sub(Ffp::from_i32(100)), Ffp::from_i32(-99));
    }

    #[test]
    fn arithmetic_matches_exact_rounding_mostly() {
        // Sanity: results stay within one unit of the exact value.
        let vals = ["0.1", "3.14159", "179.35", "-2.5", "1e10", "7", "-0.003", "123456"];
        for x in vals {
            for y in vals {
                let (fx, fy) = (ascii_to_ffp(x.as_bytes()), ascii_to_ffp(y.as_bytes()));
                let check = |r: Ffp, exact: f64| {
                    if exact.abs() > 9.2e18 {
                        return;
                    }
                    let ulp = exact.abs() * 2f64.powi(-23);
                    assert!((r.to_f64() - exact).abs() <= ulp, "{x} {y}: {:08X} {} {exact}", r.0, r.to_f64());
                };
                check(fx.add(fy), fx.to_f64() + fy.to_f64());
                check(fx.sub(fy), fx.to_f64() - fy.to_f64());
                check(fx.mul(fy), fx.to_f64() * fy.to_f64());
                // The Motorola divide corrects its first 16 bit quotient
                // estimate only once, so it can be off by more (see below).
                if fy.0 & 0xFF00 == 0 {
                    check(fx.div(fy), fx.to_f64() / fy.to_f64());
                }
            }
        }
    }

    #[test]
    fn print_free() {
        assert_eq!(format_ffp(Ffp::ONE.div(Ffp::from_i32(3)), Fix::Free), " 0.3333333");
        assert_eq!(p("0.1"), " 0.1");
        assert_eq!(p("1.5"), " 1.5");
        assert_eq!(p("3.14159"), " 3.14159");
        assert_eq!(p("100"), " 100");
        assert_eq!(p("0.5"), " 0.5");
        assert_eq!(p("-2.25"), "-2.25");
        assert_eq!(p("0"), " 0");
        assert_eq!(p("179.35"), " 179.35");
        assert_eq!(p("12345678"), " 1.23456 E+07");
        assert_eq!(p("123456789.0"), " 1.23456 E+08");
        assert_eq!(p("1e10"), " 1E+10");
        assert_eq!(p("0.0001"), " 1E-04");
        assert_eq!(p("0.00123"), " 0.00123");
        assert_eq!(p("1234567"), " 1.23456 E+06");
        assert_eq!(p("123456"), " 123456");
        assert_eq!(p("-0.5"), "-0.5");
        assert_eq!(p("2.5e-5"), " 2.5 E-05");
    }

    #[test]
    fn print_fix() {
        let pi = ascii_to_ffp(b"3.14159");
        assert_eq!(format_ffp(pi, Fix::from_fix_arg(2)), " 3.14");
        assert_eq!(format_ffp(pi, Fix::from_fix_arg(0)), " 3");
        assert_eq!(format_ffp(pi, Fix::from_fix_arg(-3)), " 3.141E+00");
        assert_eq!(format_ffp(pi, Fix::from_fix_arg(16)), " 3.14159");
        assert_eq!(format_ffp(pi, Fix::from_fix_arg(-16)), " 3.14158 E+00");
        assert_eq!(format_ffp(ascii_to_ffp(b"-1.5"), Fix::from_fix_arg(2)), "-1.50");
        assert_eq!(format_ffp(ascii_to_ffp(b"0.0001"), Fix::from_fix_arg(2)), " 0.00");
        assert_eq!(format_ffp(ascii_to_ffp(b"0.5"), Fix::from_fix_arg(0)), " 0");
        assert_eq!(format_ffp(ascii_to_ffp(b"0.25"), Fix::from_fix_arg(-2)), " 2.50E-01");
        assert_eq!(format_ffp(Ffp::ZERO, Fix::from_fix_arg(-3)), " 0.000E+00");
        assert_eq!(format_ffp(Ffp::ZERO, Fix::ExponentFree), " 0E+00");
        assert_eq!(format_ffp(ascii_to_ffp(b"12345678"), Fix::from_fix_arg(-2)), " 1.23E+07");
        assert_eq!(Fix::from_fix_arg(i32::MIN), Fix::ExponentFree);
        assert_eq!(Fix::from_fix_arg(-1), Fix::Exponent(1));
        assert_eq!(Fix::from_fix_arg(15), Fix::Decimals(15));
    }

    #[test]
    fn print_int() {
        assert_eq!(format_int(5), " 5");
        assert_eq!(format_int(-12), "-12");
        assert_eq!(format_int(0), " 0");
        assert_eq!(format_int(i32::MIN), "-2147483648");
    }

    #[test]
    fn print_double() {
        assert_eq!(format_double(3.5, Fix::Free), " 3.5");
        assert_eq!(format_double(123.0, Fix::Free), " 123");
        assert_eq!(format_double(1e20, Fix::Free), " 1e+020");
        assert_eq!(format_double(1.5e-7, Fix::Free), " 1.5e-007");
        assert_eq!(format_double(0.001, Fix::Free), " 0.001");
        assert_eq!(format_double(0.0, Fix::Free), " 0");
        assert_eq!(format_double(-2.25, Fix::Free), "-2.25");
        assert_eq!(format_double(1.0 / 3.0, Fix::Free), " 0.333333333333333");
        assert_eq!(format_double(1.25, Fix::from_fix_arg(2)), " 1.3");
        assert_eq!(format_double(123.4, Fix::from_fix_arg(-3)), " 1.234e+002");
        assert_eq!(format_double(1e15, Fix::Free), " 1e+015");
        assert_eq!(format_double(123456789012345.0, Fix::Free), " 123456789012345");
    }
}

#[cfg(test)]
mod program_tests {
    use super::*;
    use crate::program::Program;
    use crate::tokens::*;

    fn amos_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                amos_files(&path, out);
            } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("amos")) {
                out.push(path);
            }
        }
    }

    /// Every single precision constant in the example programs survives
    /// the editor's listing format followed by `a2ffp`.
    #[test]
    fn listing_round_trip_of_program_constants() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../AMOS-Professional-365");
        let mut files = Vec::new();
        amos_files(&dir, &mut files);
        let mut count = 0;
        let mut bad = Vec::new();
        for path in files {
            let Ok(prg) = Program::load(&std::fs::read(&path).unwrap()) else {
                continue;
            };
            for (_, v) in prg.lines() {
                let mut p = 2;
                while p + 2 <= v.len() {
                    let t = u16::from_be_bytes([v[p], v[p + 1]]);
                    p += 2;
                    if t == 0 {
                        break;
                    }
                    match t {
                        TK_VAR | TK_LAB | TK_PRO | TK_LGO => p += 4 + v[p + 2] as usize,
                        TK_CH1 | TK_CH2 | TK_REM1 | TK_REM2 => {
                            let n = u16::from_be_bytes([v[p], v[p + 1]]) as usize;
                            p += 2 + n + (n & 1);
                        }
                        TK_FL => {
                            let f = Ffp(u32::from_be_bytes([v[p], v[p + 1], v[p + 2], v[p + 3]]));
                            let text = format_ffp_listing(f);
                            count += 1;
                            if ascii_to_ffp(text.as_bytes()) != f {
                                bad.push(format!("{}: {:08X} {text}", path.display(), f.0));
                            }
                            p += 4;
                        }
                        TK_ENT | TK_HEX | TK_BIN => p += 4,
                        TK_DFL => p += 8,
                        TK_EXT => p += 4,
                        _ => p += inline_data_size(t),
                    }
                }
            }
        }
        assert!(count > 100, "{count}");
        assert!(bad.is_empty(), "{count} constants, failures: {bad:#?}");
    }
}
