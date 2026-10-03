//! Reference for `softdouble.rs` (tests only): the double precision
//! routines of `+Lib.s` transcribed instruction by instruction, and
//! `AscToDouble` (`L3D9C0`) transcribed label by label.
//!
//! `AscToDouble` (`+Lib.s:27066`, the routine at `L3D9C0`) converts text with
//! the IEEE double routines of the C runtime linked into `+Lib.s` (after
//! `DoubleToAsc`), not with `mathieeedoubbas`: add `L3DE80`, multiply
//! `L3DFA0`, divide `L3E12A`, long -> double `L3DDA6`, compare `L3DDC6`,
//! negate `L3DE4E`, all ending in the normalise / round / pack routine
//! `L3E26A`. They are not correctly rounded (the multiplication leaves out
//! low partial products and rounds twice, the division truncates its
//! quotient before rounding, the addition keeps a single sticky bit, and
//! values with a zero exponent field are zero), so they are ported here
//! instruction by instruction: registers are `u32`, `x` is the 68000 X flag.
//!
//! A value is a `u64`: `D0` (sign, exponent, high mantissa) in the high
//! half, `D1` in the low half.

#![allow(dead_code, unused_assignments)]

/// The value `AscToDouble` returns on overflow (`L3DAE6`), positive.
pub const OVERFLOW: u64 = super::OVERFLOW;

#[inline]
fn hi(v: u64) -> u32 {
    (v >> 32) as u32
}

#[inline]
fn lo(v: u64) -> u32 {
    v as u32
}

#[inline]
fn join(d0: u32, d1: u32) -> u64 {
    ((d0 as u64) << 32) | d1 as u64
}

#[inline]
fn swap(r: u32) -> u32 {
    r.rotate_left(16)
}

#[inline]
fn set_w(r: u32, w: u32) -> u32 {
    (r & 0xFFFF_0000) | (w & 0xFFFF)
}

#[inline]
fn exp_zero(d: u32) -> bool {
    swap(d) & 0x7FF0 == 0
}

/// `L3E26A` (`D7` = 0) / `L3E26C`: normalises the 64 bit mantissa `D0:D1`
/// (exponent `D4.w`), rounds to 53 bits (ties to even on the bits below)
/// and packs with the sign `D6.b`.
fn pack(mut d0: u32, mut d1: u32, mut d4: u16, d6: u8, d7: u32) -> u64 {
    if d0 == 0 {
        std::mem::swap(&mut d0, &mut d1);
        d4 = d4.wrapping_sub(0x20);
        if d0 == 0 {
            // L3E246
            return 0;
        }
    }
    if d0 & 0x8000_0000 == 0 {
        loop {
            d4 = d4.wrapping_sub(1);
            let x = d1 >> 31;
            d1 <<= 1;
            d0 = (d0 << 1) | x;
            if d0 & 0x8000_0000 != 0 {
                break;
            }
        }
    }
    // L3E284
    let d5 = d1 & 0x7FF;
    let round = d5 > 0x400 || (d5 == 0x400 && d1 & 0x800 != 0);
    if round {
        // L3E292: ADDI.L #$800,D1 / ADDX.L D7,D0
        let (s, c) = d1.overflowing_add(0x800);
        d1 = s;
        let (s0, c0a) = d0.overflowing_add(d7);
        let (s0, c0b) = s0.overflowing_add(c as u32);
        d0 = s0;
        if c0a || c0b {
            // ROXR with X = 1.
            let x = d0 & 1;
            d0 = (d0 >> 1) | 0x8000_0000;
            d1 = (d1 >> 1) | (x << 31);
            d4 = d4.wrapping_add(1);
        }
    }
    // L3E2AC
    let d7w = (d0 << 5) & 0xFFFF;
    d0 >>= 11;
    d1 >>= 11;
    d1 |= d7w << 16;
    d0 &= 0xF_FFFF;
    let e = d4.wrapping_add(0x3FF);
    if (e as i16) < 0 {
        // L3E2E2: underflow, the smallest normal value.
        let d0 = 0x10_0000 | if d6 != 0 { 0x8000_0000 } else { 0 };
        return join(d0, 0);
    }
    if (e as i16) > 0x7FF {
        // L3E24A
        return overflow_value(d6);
    }
    let mut e = e;
    if d6 != 0 {
        e |= 0x800;
    }
    e <<= 4;
    join(d0 | ((e as u32) << 16), d1)
}

/// `L3E252`: `D1` = -1, `D0` = $7FFFFFFF with the sign of `D6.b`.
fn overflow_value(d6: u8) -> u64 {
    join(0x7FFF_FFFF | if d6 != 0 { 0x8000_0000 } else { 0 }, 0xFFFF_FFFF)
}

/// `L3DDA6`: long -> double.
pub fn from_long(v: i32) -> u64 {
    if v == 0 {
        return 0;
    }
    let d6 = if v < 0 { 0xFF } else { 0 };
    let d0 = if v > 0 { v as u32 } else { (v as u32).wrapping_neg() };
    pack(d0, 0, 0x1F, d6, 0)
}

/// `L3DE4E`: negation (zero exponent field: +0).
pub fn neg(v: u64) -> u64 {
    if exp_zero(hi(v)) { 0 } else { v ^ (1 << 63) }
}

/// `L3DE1C`: sign test (-1, 0, 1); a zero exponent field is 0.
pub fn test(v: u64) -> i32 {
    if exp_zero(hi(v)) {
        0
    } else if hi(v) & 0x8000_0000 != 0 {
        -1
    } else {
        1
    }
}

/// `L3DDC6`: compares `a` (`D0:D1`) with `b` (`D2:D3`): -1, 0 or 1.
pub fn cmp(a: u64, b: u64) -> i32 {
    let (mut d0, mut d1, mut d2, mut d3) = (hi(a), lo(a), hi(b), lo(b));
    if exp_zero(d0) {
        d0 = 0;
        d1 = 0;
    }
    if exp_zero(d2) {
        d2 = 0;
        d3 = 0;
    }
    if d2 & 0x8000_0000 != 0 {
        // L3DE12
        if d0 & 0x8000_0000 == 0 {
            return 1;
        }
        std::mem::swap(&mut d1, &mut d3);
        // CMP.L D0,D2 then L3DDF0.
        return cmp_tail(d2 as i32, d0 as i32, d1, d3);
    }
    if d0 & 0x8000_0000 != 0 {
        return -1;
    }
    cmp_tail(d0 as i32, d2 as i32, d1, d3)
}

/// `L3DDF0`: flags of a signed `a - b`, then of an unsigned `c - d`.
fn cmp_tail(a: i32, b: i32, c: u32, d: u32) -> i32 {
    if a < b {
        return -1;
    }
    if a > b {
        return 1;
    }
    if c < d {
        -1
    } else if c > d {
        1
    } else {
        0
    }
}

/// Mantissa of a normalised operand shifted to the top of 64 bits (`ANDI.L
/// #$FFFFF / ORI.L #$100000 / LSL.L #11` and the 11 bits from the low
/// word).
fn mant64(d0: u32, d1: u32) -> (u32, u32) {
    let h = ((d0 & 0xF_FFFF) | 0x10_0000) << 11;
    let h = set_w(h, (h & 0xFFFF) | ((swap(d1) & 0xFFFF) >> 5));
    (h, d1 << 11)
}

/// `L3DE80`: `a` (`D0:D1`) + `b` (`D2:D3`).
pub fn add(a: u64, b: u64) -> u64 {
    let (d0, d1, d2, d3) = (hi(a), lo(a), hi(b), lo(b));
    let mut d6: u32 = if d0 & 0x8000_0000 != 0 { 0xFF } else { 0 };
    if exp_zero(d0) {
        // L3DE62
        return if exp_zero(d2) { 0 } else { b };
    }
    let mut d7: u32 = if d2 & 0x8000_0000 != 0 { 0xFF } else { 0 };
    if exp_zero(d2) {
        return a;
    }
    let mut e4 = (((swap(d0) & 0x7FF0) >> 4) as u16).wrapping_sub(0x3FF);
    let mut e5 = (((swap(d2) & 0x7FF0) >> 4) as u16).wrapping_sub(0x3FF);
    let (mut d0, mut d1) = mant64(d0, d1);
    let (mut d2, mut d3) = mant64(d2, d3);
    if e4 != e5 {
        if (e4 as i16) < (e5 as i16) {
            std::mem::swap(&mut d0, &mut d2);
            std::mem::swap(&mut d1, &mut d3);
            std::mem::swap(&mut e4, &mut e5);
            std::mem::swap(&mut d6, &mut d7);
        }
        // L3DEF6
        let n = e4.wrapping_sub(e5);
        if (n.wrapping_sub(0x37) as i16) >= 0 {
            d2 = 0;
            d3 = 0;
        } else if (n.wrapping_sub(5) as i16) < 0 {
            // L3DF5A: n single bit shifts, the bits out are lost.
            for _ in 0..n {
                let x = d2 & 1;
                d2 >>= 1;
                d3 = (d3 >> 1) | (x << 31);
            }
        } else if (n.wrapping_sub(0x20) as i16) < 0 {
            // L3DF36
            let k = n as u32;
            let d5 = d3;
            d3 >>= k;
            if d3 << k != d5 {
                d3 |= 1;
            }
            let d5 = d2;
            d2 >>= k;
            d3 |= d5 << (32 - k);
        } else {
            let k = (n - 0x20) as u32;
            if d3 != 0 {
                d3 = d2 >> k;
                d3 |= 1;
            } else {
                d3 = d2 >> k;
                if d3 << k != d2 {
                    d3 |= 1;
                }
            }
            d2 = 0;
        }
    }
    // L3DF66
    let (w6, w7) = (d6 as u16 as i16, d7 as u16 as i16);
    let mut sign = d6;
    if w6 == w7 {
        let (s1, c1) = d1.overflowing_add(d3);
        let (s0, c0a) = d0.overflowing_add(d2);
        let (s0, c0b) = s0.overflowing_add(c1 as u32);
        d1 = s1;
        d0 = s0;
        if c0a || c0b {
            let x = d0 & 1;
            d0 = (d0 >> 1) | 0x8000_0000;
            d1 = (d1 >> 1) | (x << 31);
            e4 = e4.wrapping_add(1);
        }
    } else {
        if w6 > w7 {
            // L3DF7A
            std::mem::swap(&mut d0, &mut d2);
            std::mem::swap(&mut d1, &mut d3);
            sign = d7;
        }
        // L3DF80: SUB.L D3,D1 / SUBX.L D2,D0
        let (s1, b1) = d1.overflowing_sub(d3);
        let (s0, b0a) = d0.overflowing_sub(d2);
        let (s0, b0b) = s0.overflowing_sub(b1 as u32);
        d1 = s1;
        d0 = s0;
        if b0a || b0b {
            // NEGX.B D6 (X = 1), NEG.L D1, NEGX.L D0.
            sign = (sign & !0xFF) | ((0u8.wrapping_sub(sign as u8).wrapping_sub(1)) as u32);
            let x = d1 != 0;
            d1 = d1.wrapping_neg();
            d0 = 0u32.wrapping_sub(d0).wrapping_sub(x as u32);
        }
    }
    pack(d0, d1, e4, sign as u8, 0)
}

/// `MULU`: low words.
#[inline]
fn mulu(a: u32, b: u32) -> u32 {
    (a & 0xFFFF) * (b & 0xFFFF)
}

/// `ADD.L a,b` with X.
#[inline]
fn add_l(b: &mut u32, a: u32, x: &mut bool) {
    let (s, c) = b.overflowing_add(a);
    *b = s;
    *x = c;
}

/// `ADDX.L a,b`.
#[inline]
fn addx_l(b: &mut u32, a: u32, x: &mut bool) {
    let (s, c1) = b.overflowing_add(a);
    let (s, c2) = s.overflowing_add(*x as u32);
    *b = s;
    *x = c1 || c2;
}

/// `ADDX.W a,b` (low words; the high word of `b` is kept).
#[inline]
fn addx_w(b: &mut u32, a: u32, x: &mut bool) {
    let s = (*b & 0xFFFF) + (a & 0xFFFF) + *x as u32;
    *x = s > 0xFFFF;
    *b = set_w(*b, s);
}

/// `ADD.W a,b`.
#[inline]
fn add_w(b: &mut u32, a: u32, x: &mut bool) {
    let s = (*b & 0xFFFF) + (a & 0xFFFF);
    *x = s > 0xFFFF;
    *b = set_w(*b, s);
}

/// `L3DFA0`: `a` (`D0:D1`) * `b` (`D2:D3`).
pub fn mul(a: u64, b: u64) -> u64 {
    let (d0, d1, d2, d3) = (hi(a), lo(a), hi(b), lo(b));
    if exp_zero(d0) || exp_zero(d2) {
        // L3DF96
        return 0;
    }
    let s6: u8 = if d0 & 0x8000_0000 != 0 { 0xFF } else { 0 };
    let s7: u8 = if d2 & 0x8000_0000 != 0 { 0xFF } else { 0 };
    // ADD.W D5,D4 / LSR.W #4 / SUBI.W #$7FD (exponent fields << 4).
    let e = (((swap(d0) & 0x7FF0) + (swap(d2) & 0x7FF0)) & 0xFFFF) >> 4;
    let mut a0 = (e as u16).wrapping_sub(0x7FD);
    let mut a1: u32 = (s6 ^ s7) as u32;
    let (mut d0, mut d1) = mant64(d0, d1);
    let (op_hi, op_lo) = mant64(d2, d3);
    let mut x = false;
    let d7: u32 = 0;
    let a2 = op_hi;
    let mut d2 = op_lo;
    let mut d3: u32 = 0;
    let mut d4: u32 = 0;
    let mut d5: u32 = 0;
    let mut d6: u32 = 0;
    if d2 & 0xFFFF != 0 {
        d3 = set_w(d3, d0);
        if d3 & 0xFFFF != 0 {
            d3 = mulu(d2, d3);
            d3 = swap(d3);
            d6 = set_w(d6, d3);
        }
        // L3E018
        d3 = swap(d0);
        if d3 & 0xFFFF != 0 {
            d3 = mulu(d2, d3);
            add_l(&mut d6, d3, &mut x);
        }
    }
    // L3E024
    d2 = swap(d2);
    if d2 & 0xFFFF != 0 {
        d3 = swap(d1);
        if d3 & 0xFFFF != 0 {
            d3 = mulu(d2, d3);
            d3 &= 0xFFFF_0000;
            d3 = swap(d3);
            add_l(&mut d6, d3, &mut x);
            addx_w(&mut d5, d7, &mut x);
        }
        // L3E03C
        d3 = set_w(d3, d0);
        if d3 & 0xFFFF != 0 {
            d3 = mulu(d2, d3);
            add_l(&mut d6, d3, &mut x);
            addx_w(&mut d5, d7, &mut x);
        }
        // L3E046
        d3 = swap(d0);
        d3 = mulu(d2, d3);
        d6 = swap(d6);
        add_w(&mut d6, d3, &mut x);
        d6 = swap(d6);
        d3 &= 0xFFFF_0000;
        d3 = swap(d3);
        addx_l(&mut d5, d3, &mut x);
    }
    // L3E058
    d2 = a2;
    if d2 & 0xFFFF != 0 {
        d3 = set_w(d3, d1);
        if d3 & 0xFFFF != 0 {
            d3 = mulu(d2, d3);
            d3 &= 0xFFFF_0000;
            d3 = swap(d3);
            add_l(&mut d6, d3, &mut x);
            addx_l(&mut d5, d7, &mut x);
        }
        // L3E06C
        d3 = swap(d1);
        if d3 & 0xFFFF != 0 {
            d3 = mulu(d2, d3);
            add_l(&mut d6, d3, &mut x);
            addx_l(&mut d5, d7, &mut x);
        }
        // L3E07A
        d3 = set_w(d3, d0);
        if d3 & 0xFFFF != 0 {
            d3 = mulu(d2, d3);
            d6 = swap(d6);
            add_w(&mut d6, d3, &mut x);
            d6 = swap(d6);
            d3 &= 0xFFFF_0000;
            d3 = swap(d3);
            addx_l(&mut d5, d3, &mut x);
        }
        // L3E08C
        d3 = swap(d0);
        d3 = mulu(d2, d3);
        add_l(&mut d5, d3, &mut x);
        addx_l(&mut d4, d7, &mut x);
    }
    // L3E096
    d2 = swap(d2);
    d3 = set_w(d3, d1);
    if d3 & 0xFFFF != 0 {
        d3 = mulu(d2, d3);
        add_l(&mut d6, d3, &mut x);
        addx_l(&mut d5, d7, &mut x);
        addx_l(&mut d4, d7, &mut x);
    }
    // L3E0A4
    d1 = swap(d1);
    d3 = set_w(d3, d1);
    if d3 & 0xFFFF != 0 {
        d3 = mulu(d2, d3);
        d6 = swap(d6);
        add_w(&mut d6, d3, &mut x);
        d6 = swap(d6);
        d3 &= 0xFFFF_0000;
        d3 = swap(d3);
        addx_l(&mut d5, d3, &mut x);
        addx_l(&mut d4, d7, &mut x);
    }
    // L3E0BA
    d3 = set_w(d3, d0);
    if d3 & 0xFFFF != 0 {
        d3 = mulu(d2, d3);
        add_l(&mut d5, d3, &mut x);
        addx_l(&mut d4, d7, &mut x);
    }
    // L3E0C4
    d0 = swap(d0);
    d3 = set_w(d3, d0);
    d3 = mulu(d2, d3);
    d5 = swap(d5);
    add_w(&mut d5, d3, &mut x);
    d5 = swap(d5);
    d3 &= 0xFFFF_0000;
    d3 = swap(d3);
    addx_l(&mut d4, d3, &mut x);
    let shr3 = |d4: &mut u32, d5: &mut u32, d6: &mut u32| {
        let c4 = *d4 & 1;
        *d4 >>= 1;
        let c5 = *d5 & 1;
        *d5 = (*d5 >> 1) | (c4 << 31);
        *d6 = (*d6 >> 1) | (c5 << 31);
    };
    if d4 > 0xFFFF {
        a1 = a1.wrapping_add(1);
        shr3(&mut d4, &mut d5, &mut d6);
    }
    // L3E0E6: CMPI.W #$8000,D6
    let w = d6 & 0xFFFF;
    if w == 0x8000 {
        // L3E10A
        d6 |= 0x1_0000;
    } else if w > 0x8000 {
        d6 = swap(d6);
        let s = (d6 & 0xFFFF) + 1;
        x = s > 0xFFFF;
        d6 = set_w(d6, s);
        d6 = swap(d6);
        addx_l(&mut d5, d7, &mut x);
        addx_l(&mut d4, d7, &mut x);
        if d4 > 0xFFFF {
            a0 = a0.wrapping_add(1);
            shr3(&mut d4, &mut d5, &mut d6);
        }
    }
    // L3E110
    d6 = set_w(d6, d5);
    d6 = swap(d6);
    let r1 = d6;
    d5 = set_w(d5, d4);
    d5 = swap(d5);
    let r0 = d5;
    pack(r0, r1, a0, a1 as u8, 0)
}

/// `DIVU.W`: `None` on overflow (destination unchanged).
fn divu(d: u32, s: u32) -> Option<u32> {
    let s = s & 0xFFFF;
    let q = d / s;
    if q > 0xFFFF {
        return None;
    }
    Some(((d % s) << 16) | q)
}

/// `L3E12A`: `a` (`D0:D1`) / `b` (`D2:D3`).
pub fn div(a: u64, b: u64) -> u64 {
    let (d0, d1, d2, d3) = (hi(a), lo(a), hi(b), lo(b));
    if exp_zero(d0) {
        return 0;
    }
    let s6: u8 = if d0 & 0x8000_0000 != 0 { 0xFF } else { 0 };
    let s7: u8 = if d2 & 0x8000_0000 != 0 { 0xFF } else { 0 };
    if exp_zero(d2) {
        // L3E23A: division by zero, the largest value.
        return overflow_value(s6);
    }
    let e = ((swap(d0) & 0x7FF0) as u16).wrapping_sub((swap(d2) & 0x7FF0) as u16);
    let mut a0 = ((e as i16) >> 4).wrapping_sub(1) as u16;
    let sign = s6 ^ s7;
    let (mut d0, mut d1) = mant64(d0, d1);
    let (mut d2, mut d3) = mant64(d2, d3);
    let (q0, q1);
    if d2 & 0xFFFF == 0 && d3 == 0 {
        // A 16 bit divisor: DIVU.W.
        d2 = swap(d2);
        d0 = swap(d0);
        let (w0, w2) = (d0 & 0xFFFF, d2 & 0xFFFF);
        if w0 >= w2 {
            // L3E19A
            d0 = swap(d0);
            a0 = a0.wrapping_add(1);
            let x = d0 & 1;
            d0 >>= 1;
            d1 = (d1 >> 1) | (x << 31);
            d0 = swap(d0);
        }
        // L3E1A4
        d0 = swap(d0);
        let div = |d0: &mut u32| {
            if let Some(r) = divu(*d0, d2) {
                *d0 = r;
            }
        };
        div(&mut d0);
        let mut d4 = set_w(0, d0);
        d4 = swap(d4);
        d1 = swap(d1);
        d0 = set_w(d0, d1);
        div(&mut d0);
        d4 = set_w(d4, d0);
        d1 = swap(d1);
        d0 = set_w(d0, d1);
        div(&mut d0);
        let mut d5 = set_w(0, d0);
        d5 = swap(d5);
        d0 &= 0xFFFF_0000;
        div(&mut d0);
        d5 = set_w(d5, d0);
        q0 = d4;
        q1 = d5;
    } else if d0 == d2 && d1 == d3 {
        q0 = 0x8000_0000;
        q1 = 0;
        a0 = a0.wrapping_add(1);
    } else {
        // L3E1DE: BHI after the last compare (high words, else low words).
        let divisor_higher = if d2 != d0 { d2 > d0 } else { d3 > d1 };
        if !divisor_higher {
            a0 = a0.wrapping_add(1);
            let x = d0 & 1;
            d0 >>= 1;
            d1 = (d1 >> 1) | (x << 31);
        }
        // L3E1E6
        let x = d2 & 1;
        d2 >>= 1;
        d3 = (d3 >> 1) | (x << 31);
        let x = d0 & 1;
        d0 >>= 1;
        d1 = (d1 >> 1) | (x << 31);
        // Non restoring division, 2 x 32 quotient bits (L3E1F2..L3E22A).
        let mut d4: u32 = 0;
        let mut d5: u32 = 0;
        let mut subtract = true;
        for word in 0..2 {
            if word == 1 {
                d5 = d4;
            }
            d4 = 0;
            for _ in 0..32 {
                d4 <<= 1;
                let x = d1 >> 31;
                d1 <<= 1;
                d0 = (d0 << 1) | x;
                let (r1, r0) = if subtract {
                    let (s1, b1) = d1.overflowing_sub(d3);
                    (s1, d0.wrapping_sub(d2).wrapping_sub(b1 as u32))
                } else {
                    let (s1, c1) = d1.overflowing_add(d3);
                    (s1, d0.wrapping_add(d2).wrapping_add(c1 as u32))
                };
                d1 = r1;
                d0 = r0;
                if d0 & 0x8000_0000 == 0 {
                    // L3E202
                    d4 = set_w(d4, d4 + 1);
                    subtract = true;
                } else {
                    subtract = false;
                }
            }
        }
        q0 = d5;
        q1 = d4;
    }
    // L3E22E
    pack(q0, q1, a0, sign, 0)
}

/// Character classes (`DDebut+$91`): bit 2 digit, bit 4 space.
fn ctype(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => 0x0C,
        9..=13 => 0x30,
        b' ' => 0x90,
        _ => 0,
    }
}

/// `L3DCFA`: `D4` from 1 to `n`, `r = r * 10` (`L3DFA0`, `D0:D1` = r).
fn l3dcfa(n: i32) -> u64 {
    let mut r: u64 = 0x3FF0_0000_0000_0000;
    let mut d4 = 1;
    while d4 <= n {
        r = mul(r, 0x4024_0000_0000_0000);
        d4 += 1;
        if n > 2000 && r == 0x7FFF_FFFF_FFFF_FFFF {
            // (The overflow value is a fixed point: skip the rest.)
            break;
        }
    }
    r
}

/// `L3D9C0` with no end pointer (`D7` = 0).
pub fn asc_to_double(text: &[u8]) -> u64 {
    // The string, NUL terminated.
    let mut s = text.to_vec();
    s.push(0);
    let b = |p: usize| s[p];
    // -4/-8(A5): the value; -$C/-$10(A5): the previous value.
    let mut v: u64;
    let mut old: u64;
    let mut d6 = false;
    let mut a6 = 0usize;
    // L3D9D6
    while ctype(b(a6)) & 0x10 != 0 {
        a6 += 1;
    }
    if b(a6) == 0 {
        return 0;
    }
    // L3DA04 / L3DA32
    match b(a6) {
        0x2B => a6 += 1,
        0x2D => {
            d6 = true;
            a6 += 1;
        }
        c => {
            if ctype(c) & 4 == 0 && c != 0x2E {
                return 0;
            }
        }
    }
    // L3DA3E
    let mut a2 = a6;
    while ctype(b(a2)) & 4 != 0 {
        a2 += 1;
    }
    let mut a3 = a2;
    v = 0;
    // SUBQ.L #1,A2 / L3DB14: CMPA.L A6,A2 / BCC (addresses: A2 may go below A6).
    let mut a2i = a2 as isize - 1;
    while a2i >= a6 as isize {
        let a2 = a2i as usize;
        old = v;
        let p = l3dcfa((a3 - a2 - 1) as i32);
        let d = from_long((b(a2) as i32) - 0x30);
        let t = mul(p, d);
        v = add(t, v);
        if cmp(v, old) < 0 {
            return l3dae6(d6);
        }
        a2i -= 1;
    }
    let mut a2;
    if b(a3) == 0x2E {
        a2 = a3 + 1;
        // L3DB6C
        while ctype(b(a2)) & 4 != 0 {
            let d = from_long((b(a2) as i32) - 0x30);
            let p = l3dcfa((a2 - a3) as i32);
            let t = div(d, p);
            v = add(t, v);
            a2 += 1;
        }
        a3 = a2;
    } else {
        // L3DB84
        a2 = a3;
    }
    // L3DB86
    if b(a2) == 0x65 || b(a2) == 0x45 {
        if b(a2 + 1) == 0x2D || b(a2 + 1) == 0x2B {
            a2 += 1;
        }
        // L3DBB6
        a3 = a2 + 1;
        if ctype(b(a3)) & 4 != 0 {
            let mut d4: i32 = 0;
            // L3DBD2
            while ctype(b(a3)) & 4 != 0 {
                let d5 = d4;
                d4 = d4.wrapping_mul(10).wrapping_add(b(a3) as i8 as i32).wrapping_sub(0x30);
                if d4 >= d5 {
                    a3 += 1;
                    continue;
                }
                if b(a2) == 0x2D {
                    v = 0;
                    d4 = 0;
                    break;
                }
                return l3dae6(d6);
            }
            // L3DC34
            if b(a2) == 0x2D {
                let p = l3dcfa(d4);
                v = div(v, p);
            } else {
                // L3DC90
                old = v;
                let p = l3dcfa(d4);
                v = mul(p, v);
                if cmp(v, old) < 0 {
                    return l3dae6(d6);
                }
            }
        }
    }
    // L3DCCE
    if d6 {
        v = neg(v);
    }
    v
}

/// `L3DAE6`: overflow.
fn l3dae6(d6: bool) -> u64 {
    if d6 { OVERFLOW | (1 << 63) } else { OVERFLOW }
}
