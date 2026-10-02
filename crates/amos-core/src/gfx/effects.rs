//! Colour effects run by the VBL interrupt (Flash, Shift, Fade, +W.s:5263-5695)
//! and rainbows (+W.s:3889-4161, copper merge 6052-6170).

use super::screen::Screens;

/// Maximum number of flashing colours (`FlMax`).
pub const FLASH_MAX: usize = 16;
/// Number of rainbows (`NbRain`).
pub const RAINBOWS: usize = 4;

/// One flashing colour (`T_tflash` entry).
#[derive(Clone, Debug, Default)]
pub struct Flash {
    pub screen: usize,
    pub colour: usize,
    pub counter: u16,
    pub pos: usize,
    /// (delay in VBLs, $RGB) pairs.
    pub steps: Vec<(u16, u16)>,
}

/// `Shift Up/Down` state (`T_TShift`).
#[derive(Clone, Debug)]
pub struct Shift {
    pub screen: usize,
    pub counter: u16,
    pub speed: u16,
    pub c1: usize,
    pub c2: usize,
    pub down: bool,
    pub rotate: bool,
}

/// `Fade` state (`T_FadeCol`...).
#[derive(Clone, Debug)]
pub struct Fade {
    pub screen: usize,
    pub counter: u16,
    pub speed: u16,
    /// Colour index (None once reached), current and target components.
    pub colours: Vec<(Option<usize>, [u8; 3], [u8; 3])>,
}

/// A rainbow (`RnDY`... +WEqu.s:142).
#[derive(Clone, Debug, Default)]
pub struct Rainbow {
    /// Colour register replaced (0-15).
    pub colour: usize,
    /// One $RGB value per line (`RnBuf`).
    pub buf: Vec<u16>,
    /// `Rainbow n,base,y,h` values (`RnX`, `RnY`, `RnI`); h < 0 = not shown.
    pub base: i32,
    pub y: i32,
    pub height: i32,
}

impl Rainbow {
    /// Colour shown on hardware line `line`, if the rainbow covers it
    /// (`CopBow`, +W.s:6053: first line at least 28).
    pub fn colour_at(&self, line: i32) -> Option<u16> {
        if self.buf.is_empty() || self.height < 0 {
            return None;
        }
        let dy = self.y.max(28);
        if line < dy || line >= dy + self.height {
            return None;
        }
        let len = self.buf.len() as i32;
        let base = if (0..len).contains(&self.base) {
            self.base
        } else {
            0
        };
        Some(self.buf[((base + line - dy) % len) as usize])
    }
}

#[derive(Clone, Debug, Default)]
pub struct Effects {
    pub flashes: [Option<Flash>; FLASH_MAX],
    pub shift: Option<Shift>,
    pub fade: Option<Fade>,
    pub rainbows: [Rainbow; RAINBOWS],
}

impl Effects {
    /// Stops the effects of a screen being closed (`FlStop`, `ShStop`,
    /// `FaStop` in `EcDel`).
    pub fn stop_screen(&mut self, n: usize) {
        self.flash_off(n);
        if self.shift.as_ref().is_some_and(|s| s.screen == n) {
            self.shift = None;
        }
        if self.fade.as_ref().is_some_and(|f| f.screen == n) {
            self.fade = None;
        }
    }

    /// `Flash Off` (`FlStop`): stops the flashes of screen `n`.
    pub fn flash_off(&mut self, n: usize) {
        for f in self.flashes.iter_mut() {
            if f.as_ref().is_some_and(|f| f.screen == n) {
                *f = None;
            }
        }
    }

    /// `Flash colour,a$` (`FlStart`, +W.s:5281). Errors: 7 too many
    /// flashes, 8 syntax error.
    pub fn flash(&mut self, screen: usize, colour: usize, def: &[u8]) -> Result<(), u16> {
        if self.flashes.iter().filter(|f| f.is_some()).count() >= FLASH_MAX {
            return Err(7);
        }
        // First free entry, or the entry of the same colour (in order).
        let slot = self.flashes.iter().position(|f| match f {
            None => true,
            Some(f) => f.colour == colour && f.screen == screen,
        });
        let Some(slot) = slot else { return Err(8) };
        self.flashes[slot] = None;
        if def.first().copied().unwrap_or(0) == 0 {
            return Ok(());
        }
        match parse_flash(def) {
            Some(steps) => {
                self.flashes[slot] = Some(Flash {
                    screen,
                    colour,
                    counter: 1,
                    pos: 0,
                    steps,
                });
                Ok(())
            }
            None => Err(8),
        }
    }
}

/// Parses `(RGB,delay)(RGB,delay)...` like `FlStart`.
fn parse_flash(s: &[u8]) -> Option<Vec<(u16, u16)>> {
    let mut i = 0;
    // `miniget`: skips spaces, upper cases letters; 0 at the end.
    let get = |i: &mut usize| -> u8 {
        loop {
            let c = s.get(*i).copied().unwrap_or(0);
            if c != 0 {
                *i += 1;
            }
            if c == b' ' {
                continue;
            }
            return if c >= b'a' { c - 32 } else { c };
        }
    };
    let mut steps = Vec::new();
    loop {
        let c = s.get(i).copied().unwrap_or(0);
        if c == 0 {
            break;
        }
        i += 1;
        if c != b'(' {
            return None;
        }
        if steps.len() >= 16 {
            return None;
        }
        let mut rgb = 0u16;
        for _ in 0..3 {
            let c = get(&mut i);
            if c == 0 {
                return None;
            }
            let mut d = c.wrapping_sub(b'0');
            if d > 9 {
                d = d.wrapping_sub(7);
            }
            if d > 15 {
                return None;
            }
            rgb = (rgb << 4) | d as u16;
        }
        if s.get(i).copied() != Some(b',') {
            return None;
        }
        i += 1;
        // `dechexa`: decimal number, the character after it must be ')'.
        let mut c = get(&mut i);
        let neg = c == b'-';
        if neg {
            c = get(&mut i);
        }
        if !c.is_ascii_digit() {
            return None;
        }
        let mut n: u16 = 0;
        while c.is_ascii_digit() {
            n = n.wrapping_mul(10).wrapping_add((c - b'0') as u16);
            c = get(&mut i);
        }
        if neg {
            n = n.wrapping_neg();
        }
        if n == 0 || c != b')' {
            return None;
        }
        steps.push((n, rgb));
    }
    Some(steps)
}

/// Rainbow component generator (`Trs1`, +W.s:4040): (speed, step, count)
/// triples, terminated by a negative speed.
fn rain_tok(s: &[u8]) -> Option<Vec<(i32, i32, i32)>> {
    let mut p = Parser { s, i: 0 };
    let mut out = Vec::new();
    loop {
        let c = p.chr();
        if c == 0 {
            break;
        }
        if c != b'(' {
            return None;
        }
        let speed = p.long()?;
        if speed <= 0 || p.chr() != b',' {
            return None;
        }
        let step = p.long()?;
        if p.chr() != b',' {
            return None;
        }
        let count = p.long()?;
        if count < 0 || p.chr() != b')' {
            return None;
        }
        out.push((speed, step, count));
    }
    Some(out)
}

/// `AniChr` / `AniLong` (+W.s:7040): skips spaces and lower case letters.
struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn chr(&mut self) -> u8 {
        loop {
            let c = self.s.get(self.i).copied().unwrap_or(0);
            if c == 0 {
                return 0;
            }
            self.i += 1;
            if (33..=b'Z').contains(&c) || c == b'|' || c == b'!' {
                return c;
            }
            if c == 27 {
                self.i += 2;
            }
        }
    }

    fn long(&mut self) -> Option<i32> {
        let mut c = self.chr();
        let neg = c == b'-';
        if neg {
            c = self.chr();
        }
        let mut n: i32 = 0;
        if c == b'$' {
            let mut any = false;
            loop {
                let save = self.i;
                let c = self.chr();
                let d = match c {
                    b'0'..=b'9' => c - b'0',
                    b'A'..=b'F' => c - b'A' + 10,
                    _ => {
                        self.i = save;
                        break;
                    }
                };
                any = true;
                n = n.wrapping_mul(16).wrapping_add(d as i32);
            }
            if !any {
                return None;
            }
        } else {
            if !c.is_ascii_digit() {
                return None;
            }
            n = (c - b'0') as i32;
            loop {
                let save = self.i;
                let c = self.chr();
                if !c.is_ascii_digit() {
                    self.i = save;
                    break;
                }
                n = n.wrapping_mul(10).wrapping_add((c - b'0') as i32);
            }
        }
        Some(if neg { -n } else { n })
    }
}

/// Builds the table of a rainbow (`TRSet`): `start` is the initial $RGB.
/// Returns None on a syntax error.
pub fn rainbow_table(lines: usize, start: u16, r: &[u8], g: &[u8], b: &[u8]) -> Option<Vec<u16>> {
    struct Comp {
        value: i32,
        plus: i32,
        cpt: i32,
        speed: i32,
        nb: i32,
        pos: usize,
        table: Vec<(i32, i32, i32)>,
    }
    let mk = |s: &[u8], v: u16| -> Option<Comp> {
        let mut table = rain_tok(s)?;
        if table.is_empty() {
            // An empty definition leaves a zero triple (no terminator).
            table.push((0, 0, 0));
        }
        Some(Comp {
            value: (v & 15) as i32,
            plus: 0,
            cpt: 1,
            speed: 0,
            nb: 1,
            pos: 0,
            table,
        })
    };
    let mut comps = [mk(r, start >> 8)?, mk(g, start >> 4)?, mk(b, start)?];
    let mut out = Vec::with_capacity(lines);
    for _ in 0..lines {
        for c in comps.iter_mut() {
            if c.cpt == 0 {
                continue;
            }
            c.cpt -= 1;
            if c.cpt != 0 {
                continue;
            }
            c.cpt = c.speed;
            c.value = (c.value + c.plus) & 15;
            if c.nb == 0 {
                continue;
            }
            c.nb -= 1;
            if c.nb != 0 {
                continue;
            }
            if c.pos >= c.table.len() {
                c.pos = 0;
            }
            let (speed, step, count) = c.table[c.pos];
            c.cpt = speed;
            c.speed = speed;
            c.plus = step;
            c.nb = count;
            c.pos += 1;
        }
        out.push(((comps[0].value << 8) | (comps[1].value << 4) | comps[2].value) as u16);
    }
    Some(out)
}

impl Screens {
    /// Colour effects done by the VBL interrupt, in the original order:
    /// shifter, flash, fade (+W.s:10436).
    pub fn effects_vbl(&mut self) {
        // Shift
        if let Some(sh) = self.effects.shift.as_mut() {
            sh.counter = sh.counter.wrapping_sub(1);
            if sh.counter == 0 {
                sh.counter = sh.speed;
                let sh = sh.clone();
                if let Some(s) = self.get_mut(sh.screen) {
                    let p = &mut s.palette;
                    if sh.down {
                        let first = p[sh.c1];
                        for i in sh.c1..sh.c2 {
                            p[i] = p[i + 1];
                        }
                        if sh.rotate {
                            p[sh.c2] = first;
                        }
                    } else {
                        let last = p[sh.c2];
                        for i in (sh.c1 + 1..=sh.c2).rev() {
                            p[i] = p[i - 1];
                        }
                        if sh.rotate {
                            p[sh.c1] = last;
                        }
                    }
                }
            }
        }
        // Flash
        for i in 0..FLASH_MAX {
            let Some(f) = self.effects.flashes[i].as_mut() else {
                continue;
            };
            f.counter = f.counter.wrapping_sub(1);
            if f.counter != 0 {
                continue;
            }
            if f.pos >= f.steps.len() {
                f.pos = 0;
            }
            let Some(&(delay, rgb)) = f.steps.get(f.pos) else {
                continue;
            };
            f.counter = delay;
            f.pos += 1;
            let (screen, colour) = (f.screen, f.colour);
            if let Some(s) = self.get_mut(screen)
                && colour < 32
            {
                s.palette[colour] = rgb;
            }
        }
        // Fade
        let mut stop = false;
        if let Some(fd) = self.effects.fade.as_mut() {
            fd.counter = fd.counter.wrapping_sub(1);
            if fd.counter == 0 {
                fd.counter = fd.speed;
                let mut changed = 0;
                let screen = fd.screen;
                let mut writes = Vec::new();
                for (idx, cur, target) in fd.colours.iter_mut() {
                    let Some(i) = *idx else { continue };
                    let mut moved = false;
                    for k in 0..3 {
                        if cur[k] < target[k] {
                            cur[k] += 1;
                            moved = true;
                        } else if cur[k] > target[k] {
                            cur[k] -= 1;
                            moved = true;
                        }
                    }
                    if moved {
                        changed += 1;
                        writes.push((
                            i,
                            ((cur[0] as u16) << 8) | ((cur[1] as u16) << 4) | cur[2] as u16,
                        ));
                    } else {
                        *idx = None;
                    }
                }
                if let Some(s) = self.get_mut(screen) {
                    for (i, c) in writes {
                        s.palette[i] = c;
                    }
                }
                stop = changed == 0;
            }
        }
        if stop {
            self.effects.fade = None;
        }
    }

    /// `Fade` (`FadeTOn`, +W.s:5515): fades the current screen towards
    /// `target` (negative entries are left alone), one step every `speed`
    /// VBLs.
    pub fn start_fade(&mut self, speed: u16, target: &[i32; 32]) {
        self.effects.fade = None;
        let Some(n) = self.current else { return };
        let Some(s) = self.get(n) else { return };
        let mut colours = Vec::new();
        for (i, &t) in target.iter().enumerate() {
            if t < 0 {
                continue;
            }
            let c = s.palette[i];
            let cur = [(c >> 8) as u8 & 15, (c >> 4) as u8 & 15, c as u8 & 15];
            let t = t as u16;
            let tg = [(t >> 8) as u8 & 15, (t >> 4) as u8 & 15, t as u8 & 15];
            if cur != tg {
                colours.push((Some(i), cur, tg));
            }
        }
        if !colours.is_empty() {
            self.effects.fade = Some(Fade {
                screen: n,
                counter: 1,
                speed,
                colours,
            });
        }
    }

    /// `Shift Up/Down` (`ShStart`, +W.s:5391). Error 9 if c2 <= c1.
    pub fn start_shift(
        &mut self,
        speed: i32,
        c1: i32,
        c2: i32,
        down: bool,
        rotate: bool,
    ) -> Result<(), u16> {
        let n = self.current.ok_or(3u16)?;
        self.effects.shift = None;
        let (c1, c2) = ((c1 & 31) as usize, (c2 & 31) as usize);
        if c2 <= c1 {
            return Err(9);
        }
        self.effects.shift = Some(Shift {
            screen: n,
            counter: 1,
            speed: speed as u16,
            c1,
            c2,
            down,
            rotate,
        });
        Ok(())
    }

    /// Palette of `screen` for hardware line `line` with the rainbows
    /// applied.
    pub fn line_palette(&self, palette: &[u16; 32], line: i32) -> [u16; 32] {
        let mut p = *palette;
        for r in &self.effects.rainbows {
            if let Some(c) = r.colour_at(line) {
                p[r.colour] = c;
            }
        }
        p
    }

    /// True if a rainbow covers some line of `lines`.
    pub fn rainbow_in(&self, lines: std::ops::Range<i32>) -> bool {
        self.effects
            .rainbows
            .iter()
            .any(|r| lines.clone().any(|l| r.colour_at(l).is_some()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flash_parsing() {
        assert_eq!(
            parse_flash(b"(000,2)(f0a,10)"),
            Some(vec![(2, 0x000), (10, 0xF0A)])
        );
        assert_eq!(parse_flash(b"(000,0)"), None);
        assert_eq!(parse_flash(b"(000,2) (111,2)"), None);
        let mut e = Effects::default();
        assert_eq!(e.flash(0, 3, b"(000,2"), Err(8));
        assert!(e.flash(0, 3, b"(000,2)(fff,2)").is_ok());
        assert!(e.flashes[0].is_some());
        assert!(e.flash(0, 3, b"").is_ok());
        assert!(e.flashes[0].is_none());
    }

    #[test]
    fn rainbow_generation() {
        // Red rises by one every line; blue every other line.
        let t = rainbow_table(8, 0x000, b"(1,1,3)", b"", b"(2,1,8)").unwrap();
        assert_eq!(
            t,
            vec![0x000, 0x100, 0x201, 0x301, 0x402, 0x502, 0x603, 0x703]
        );
        assert!(rainbow_table(8, 0, b"(0,1,3)", b"", b"").is_none());
    }

    #[test]
    fn fade_and_shift() {
        let mut ss = Screens::new();
        ss.open(0, 320, 200, 16, 0).unwrap();
        ss.get_mut(0).unwrap().palette[1] = 0x000;
        let mut target = [-1; 32];
        target[1] = 0x00F;
        ss.start_fade(1, &target);
        for _ in 0..20 {
            ss.effects_vbl();
        }
        assert_eq!(ss.get(0).unwrap().palette[1], 0x00F);
        assert!(ss.effects.fade.is_none());
        let p0 = ss.get(0).unwrap().palette;
        ss.start_shift(1, 1, 3, false, true).unwrap();
        ss.effects_vbl();
        let p = ss.get(0).unwrap().palette;
        assert_eq!((p[1], p[2], p[3]), (p0[3], p0[1], p0[2]));
    }
}
