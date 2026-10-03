//! The original conversion algorithms, kept as the reference for the
//! shortcuts of `ffp.rs` (tests only): literal ports of `a2ffp`, `ffp2a`,
//! `F2a`, `Clean`, `ExFix1`, `ExVir1` and `FloatToAsc`.

#![allow(dead_code)]

use std::cmp::Ordering;

use super::{
    Ffp, HALF, ONE, TEN, TWO, TWO24, ffp_add, ffp_cmp, ffp_div, ffp_from_long, ffp_mul, ffp_neg, ffp_sub, ffp_to_long,
    lb,
};

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
    // L280A8: digits of the mantissa.
    let mut v = 0u32;
    let mut p = mbuf;
    while (frame[p] as i8) >= b'0' as i8 && frame[p] <= b'9' {
        v = ffp_mul(v, TEN);
        v = ffp_add(ffp_from_long((frame[p] - b'0') as i32), v);
        p += 1;
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
    // L28040: 10^pw.
    let mut scale = ONE;
    if pw < 0 {
        for _ in 0..-(pw as i32) {
            scale = ffp_div(scale, TEN);
        }
    } else {
        for _ in 0..pw {
            scale = ffp_mul(scale, TEN);
        }
    }
    let r = renormalise(ffp_mul(scale, v));
    Ffp(if neg { r | 0x80 } else { r })
}

/// `L28116`: rebuilds an FFP value through an integer mantissa.
pub fn renormalise(x: u32) -> u32 {
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

/// `ffp2a` (`+Lib.s:26226`): C style ftoa computed in FFP.
pub fn ffp2a(x: u32, prec: i16) -> Vec<u8> {
    let mut out = Vec::new();
    let mut x = x;
    let mut ndig: i16 = if prec <= 0 {
        1
    } else if prec > 22 {
        23
    } else {
        prec + 1
    };
    let mut e: i16 = 0;
    if ffp_cmp(x, 0) == Ordering::Less {
        out.push(b'-');
        x = ffp_neg(x);
    }
    if ffp_cmp(x, 0) == Ordering::Greater {
        while ffp_cmp(x, ONE) == Ordering::Less {
            x = ffp_mul(x, TEN);
            e -= 1;
        }
    }
    while ffp_cmp(x, TEN) != Ordering::Less {
        x = ffp_div(x, TEN);
        e += 1;
    }
    ndig = ndig.wrapping_add(e);
    let mut r = ffp_from_long(1);
    let mut i: i16 = 1;
    while i < ndig {
        r = ffp_div(r, TEN);
        i += 1;
    }
    x = ffp_add(x, ffp_div(r, TWO));
    if ffp_cmp(x, TEN) != Ordering::Less {
        x = ONE;
        e += 1;
    }
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
        x = ffp_sub(x, ffp_from_long(d as i32));
        x = ffp_mul(x, TEN);
        i += 1;
    }
    out
}

/// `F2a` (`+Lib.s:25976`): `fix` = FixFlg word, `exp` = ExpFlg word.
pub fn f2a(x: u32, fix: i16, exp: i16) -> Vec<u8> {
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
    let s = ffp2a(x, prec);
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
        let n = (p - a1) as i16;
        if exp != 0 || n >= 8 {
            return ex_fix(x, n, fix);
        }
        let d = (7 - n).min(5);
        return clean(x, d);
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
    let mut z = (p - start) as i16;
    let mut zero = false;
    if z >= 22 {
        z = 6;
        zero = true;
    } else if z >= 4 {
        return ex_vir(x, z, fix, false);
    }
    if exp != 0 {
        return ex_vir(x, z, fix, zero);
    }
    clean(x, z + 6)
}

/// `Clean`: ffp2a then strip trailing zeros (and a bare point).
pub fn clean(x: u32, dec: i16) -> Vec<u8> {
    let s = ffp2a(x, dec);
    if let Some(dot) = s.iter().position(|&c| c == b'.') {
        let mut end = dot;
        for (k, &c) in s.iter().enumerate().skip(dot + 1) {
            if c != b'0' {
                end = k + 1;
            }
        }
        s[..end].to_vec()
    } else {
        s
    }
}

pub fn push_exp(out: &mut Vec<u8>, sign: u8, mut d2: i16) {
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
pub fn ex_fix(x: u32, n: i16, fix: i16) -> Vec<u8> {
    let d2 = n - 2;
    let n2 = n.min(7);
    let s = ffp2a(x, 9 - n2);
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
pub fn ex_vir(x: u32, z: i16, fix: i16, zero: bool) -> Vec<u8> {
    let (s, d2) = if zero { (b"0.0000000".to_vec(), 0) } else { (ffp2a(x, z + 6), z) };
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
pub fn float_to_asc(x: u32, fix: i16, exp: i16) -> Vec<u8> {
    float_to_asc_flags(x, fix, exp, true)
}

pub fn float_to_asc_flags(x: u32, fix: i16, exp: i16, space: bool) -> Vec<u8> {
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
