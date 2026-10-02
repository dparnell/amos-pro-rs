//! Active zones of the dialogs: drawing, the line editor, the user
//! interaction (`Dia_Tests` `+Lib.s:24133`), `Dialog Update`
//! (`Dia_ZUpdate` `+Lib.s:23887`) and erasing a dialog (`Dia_EffChanA0`
//! `+Lib.s:20697`).

use super::engine::{DR, dec, int};
use super::*;
use crate::input::KeyPress;
use crate::interp::Interp;
use crate::machine::Hardware;

/// Fake cursor of the line editor (`LEd_FauCu`): an inverse space in XOR
/// mode, then cursor left. Printed twice it disappears.
const FAKE_CURSOR: &[u8] = &[
    27, b'I', b'1', 27, b'W', b'2', b' ', 27, b'W', b'0', 27, b'I', b'0', 29,
];

/// Qualifier groups compared by the key shortcuts (`+Equ.s:775`).
const SHF: u8 = 0b0000_0011;
const CTR: u8 = 0b0000_1000;
const ALT: u8 = 0b0011_0000;
const AMI: u8 = 0b1100_0000;

/// `LEd_Lettre`: letters, digits and characters >= 128.
fn is_letter(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c >= 128
}

impl Hardware {
    /// `Dia_ClearKey`: empties the keyboard buffer and `T_ClLast`.
    pub(crate) fn dia_clear_key(&mut self) {
        self.input.clear_keys();
        self.dialogs.cl_last = None;
        self.dialogs.key_serial = self.input.key_serial;
    }

    /// `T_ClLast` is set by the keyboard interrupt for every key.
    fn dia_update_last_key(&mut self) {
        if self.input.key_serial != self.dialogs.key_serial {
            self.dialogs.key_serial = self.input.key_serial;
            self.dialogs.cl_last = Some(self.input.last_key);
        }
    }

    fn dia_timer(&mut self, ch: &mut Channel) {
        if ch.timer != 0 {
            ch.timer_pos = self.vbl_count;
        }
    }

    // ------------------------------------------------------------------
    // Text window helpers
    // ------------------------------------------------------------------

    fn dia_print(&mut self, ch: &Channel, s: &[u8]) {
        if let Some(sc) = self.screens.get_mut(ch.screen) {
            let _ = sc.print_text(s);
        }
    }

    fn dia_locate(&mut self, ch: &Channel, x: i32, y: i32) -> bool {
        self.screens
            .get_mut(ch.screen)
            .is_some_and(|s| s.locate(Some(x), Some(y)).is_ok())
    }

    fn dia_cursor(&self, ch: &Channel) -> (i32, i32) {
        self.screens
            .get(ch.screen)
            .map_or((0, 0), |s| s.cursor_pos())
    }

    /// `WPrint3`: prints `t` without control codes (TAB excepted) from the
    /// cursor column up to column `max`. Returns the column reached.
    fn dia_print3(&mut self, ch: &Channel, t: &[u8], max: i32) -> i32 {
        let mut col = self.dia_cursor(ch).0;
        let mut out = Vec::new();
        let mut i = 0;
        while i < t.len() {
            let c = t[i];
            i += 1;
            if c == 0 {
                break;
            }
            if c >= 32 {
                if col >= max {
                    break;
                }
                out.push(c);
                col += 1;
            } else if c == 9 {
                out.push(c);
            } else if c == 27 {
                i += 2;
            }
        }
        self.dia_print(ch, &out);
        col
    }

    // ------------------------------------------------------------------
    // Edit zones and the line editor
    // ------------------------------------------------------------------

    fn led(ch: &mut Channel, idx: usize) -> Option<&mut LineEd> {
        match ch.zone_mut(idx).map(|z| &mut z.kind) {
            Some(ZoneKind::Edit { led, .. }) => Some(led),
            _ => None,
        }
    }

    /// `Dia_EdOn`: activates the window of an edit zone.
    fn dia_ed_on(&mut self, ch: &mut Channel, idx: usize) {
        if let Some(z) = ch.zone(idx) {
            let n = z.number as i32 + 1000;
            self.dia_wactive(ch, n);
        }
    }

    /// `Dia_EdActive`: window and cursor of an edit zone.
    pub(crate) fn dia_ed_active(&mut self, ch: &mut Channel, idx: usize) {
        self.dia_ed_on(ch, idx);
        self.led_cursor(ch, idx, true);
    }

    fn dia_ed_inactive(&mut self, ch: &mut Channel, idx: usize) {
        self.led_cursor(ch, idx, false);
    }

    /// `LEd_CuMarche` / `LEd_CuStoppe`.
    fn led_cursor(&mut self, ch: &mut Channel, idx: usize, on: bool) {
        let Some(l) = Self::led(ch, idx) else { return };
        let shown = l.state & 4 != 0;
        if shown == on {
            return;
        }
        if on {
            l.state |= 4;
        } else {
            l.state &= !4;
        }
        self.dia_print(ch, FAKE_CURSOR);
    }

    /// `Dia_EdFirst`: the first edit zone gets the keys.
    pub(crate) fn dia_ed_first(&mut self, ch: &mut Channel) {
        ch.edited = Edited::None;
        self.dia_ed_from(ch, 0);
    }

    /// `EdaLoop`: activates the first edit zone at or after `from`.
    fn dia_ed_from(&mut self, ch: &mut Channel, from: usize) {
        let found = (from..ch.records.len())
            .find(|&i| matches!(ch.zone(i).map(|z| &z.kind), Some(ZoneKind::Edit { .. })));
        if let Some(i) = found {
            ch.edited = Edited::Zone(i);
            self.dia_ed_active(ch, i);
        }
    }

    /// `LEd_Init` (the window of the zone is current).
    pub(crate) fn dia_led_init(&mut self, ch: &mut Channel, idx: usize) -> bool {
        let (cx, cy) = self.dia_cursor(ch);
        let (tx, scroll) = self
            .screens
            .get(ch.screen)
            .and_then(|s| s.text.windows.first())
            .map_or((0, false), |w| (w.tx, w.scroll));
        let screen = ch.screen;
        let Some(l) = Self::led(ch, idx) else {
            return false;
        };
        l.x = cx;
        l.y = cy;
        l.large = tx - cx - 1 - scroll as i32;
        if l.large < 0 {
            return false;
        }
        l.state &= !4;
        l.screen = screen;
        let text = l.buf.clone();
        self.dia_print(ch, b"\x1bC0");
        self.led_new(ch, idx, &text);
        true
    }

    /// `LEd_New`: replaces the edited text (copied up to a zero or the
    /// maximum length), cursor at the end.
    fn led_new(&mut self, ch: &mut Channel, idx: usize, t: &[u8]) {
        let Some(l) = Self::led(ch, idx) else { return };
        let n = t.iter().take(l.max).take_while(|&&c| c != 0).count();
        l.buf = t[..n].to_vec();
        l.cur = n as i32;
        l.start = 0;
        l.state |= 2;
        self.led_print(ch, idx);
        self.led_cursor(ch, idx, false);
    }

    /// `LEd_Print`: redraws the visible part of the line.
    fn led_print(&mut self, ch: &mut Channel, idx: usize) {
        self.led_cursor(ch, idx, false);
        let Some(l) = Self::led(ch, idx) else { return };
        let (x, y) = (l.x, l.y);
        let clear = l.state & 2 != 0;
        l.state &= !2;
        let large = l.large;
        if l.cur < l.start {
            l.start = l.cur;
        }
        if l.cur > l.start + l.large {
            l.start = l.cur - l.large;
            l.state |= 1;
        }
        let start = l.start as usize;
        let mut n = l.buf.len() as i32 - l.start;
        if n > l.large {
            l.state &= !1;
            n = l.large + 1;
        }
        let text: Vec<u8> = l
            .buf
            .get(start..start + n.max(0) as usize)
            .unwrap_or(&[])
            .to_vec();
        let erase = l.state & 1 != 0;
        l.state &= !1;
        let cx = x + l.cur - l.start;
        self.dia_locate(ch, x, y);
        if clear {
            self.dia_print(ch, &vec![b' '; large.max(0) as usize]);
            self.dia_locate(ch, x, y);
        }
        self.dia_print(ch, &text);
        if erase {
            self.dia_print(ch, b" ");
        }
        self.dia_locate(ch, cx, y);
        self.led_cursor(ch, idx, true);
    }

    /// `LEd_Loop` with the dialog flags (one call per test): handles the
    /// mouse and the keys waiting. Returns the key left to the caller and
    /// the exit code (0 nothing / key, 1 Return, 2 mouse outside).
    fn led_loop(&mut self, ch: &mut Channel, idx: usize) -> (Option<KeyPress>, i32) {
        self.led_cursor(ch, idx, true);
        loop {
            let Some(l) = Self::led(ch, idx) else {
                return (None, 0);
            };
            let flags = l.flags;
            if self.input.mouse_buttons & 3 != 0 {
                let (mx, my) = (self.input.mouse_x, self.input.mouse_y);
                let inside = flags & led::MOUSE_CURSOR != 0
                    && self
                        .screens
                        .screen_at(mx, my, None, crate::gfx::screen::MAX_SCREENS)
                        == Some(l.screen)
                    && self.screens.get(l.screen).is_some_and(|s| {
                        let (sx, sy) = (s.x_screen(mx), s.y_screen(my));
                        match (s.x_text(sx), s.y_text(sy)) {
                            (Some(cx), Some(cy)) => {
                                let d = cx - l.x;
                                cy == l.y && d >= 0 && d + l.start <= l.buf.len() as i32
                            }
                            _ => false,
                        }
                    });
                if inside {
                    let s = self.screens.get(l.screen).unwrap();
                    let cx = s.x_text(s.x_screen(mx)).unwrap_or(0);
                    let p = cx - l.x + l.start;
                    if p != l.cur {
                        l.cur = p;
                        self.led_print(ch, idx);
                        continue;
                    }
                } else if flags & led::MOUSE != 0 {
                    return (None, 2);
                }
            }
            let Some(k) = self.input.inkey() else {
                // LEd_FOnce: back to the caller.
                return (None, 0);
            };
            let l = Self::led(ch, idx).unwrap();
            let (cur, long) = (l.cur, l.buf.len() as i32);
            match k.raw {
                0x4F => {
                    if cur == 0 {
                        continue;
                    }
                    if k.shift & 3 != 0 {
                        let mut d2 = cur;
                        loop {
                            d2 -= 1;
                            l.cur = d2;
                            if d2 == 0 || !is_letter(l.buf[d2 as usize - 1]) {
                                break;
                            }
                        }
                    } else if k.shift & 0x18 != 0 {
                        l.cur = 0;
                    } else {
                        l.cur -= 1;
                    }
                }
                0x4E => {
                    if cur >= long {
                        continue;
                    }
                    if k.shift & 3 != 0 {
                        let mut d2 = cur;
                        loop {
                            d2 += 1;
                            l.cur = d2;
                            if d2 >= long || !is_letter(l.buf[d2 as usize - 1]) {
                                break;
                            }
                        }
                    } else if k.shift & 0x18 != 0 {
                        l.cur = long;
                    } else {
                        l.cur += 1;
                    }
                }
                // Backspace then Delete (the shift test of the original
                // reads the wrong register: Shift+Backspace does not clear).
                0x41 | 0x46 => {
                    if k.raw == 0x41 {
                        if cur == 0 {
                            continue;
                        }
                        l.cur -= 1;
                    }
                    let c = l.cur;
                    if c >= l.buf.len() as i32 {
                        continue;
                    }
                    l.buf.remove(c as usize);
                    l.state |= 1;
                }
                _ if k.ascii == 13 => return (Some(k), 1),
                _ if k.ascii >= 32 => {
                    let c = k.ascii;
                    if flags & led::FILTER != 0 {
                        if c >= 128 {
                            return (None, 0);
                        }
                        let bit = c as u32 - 32;
                        if l.mask[(bit / 32) as usize] & (0x8000_0000 >> (bit % 32)) == 0 {
                            return (None, 0);
                        }
                    }
                    if long as usize >= l.max {
                        if cur != long || cur == 0 {
                            continue;
                        }
                        l.buf[cur as usize - 1] = c;
                    } else {
                        l.buf.insert(cur as usize, c);
                        l.cur += 1;
                    }
                }
                _ => {
                    if flags & led::KEYS != 0 {
                        return (Some(k), 0);
                    }
                    continue;
                }
            }
            self.led_print(ch, idx);
        }
    }

    // ------------------------------------------------------------------
    // Lists
    // ------------------------------------------------------------------

    fn list(ch: &Channel, idx: usize) -> Option<&ListZone> {
        match ch.zone(idx).map(|z| &z.kind) {
            Some(ZoneKind::List(l)) => Some(l),
            _ => None,
        }
    }

    fn list_mut(ch: &mut Channel, idx: usize) -> Option<&mut ListZone> {
        match ch.zone_mut(idx).map(|z| &mut z.kind) {
            Some(ZoneKind::List(l)) => Some(l),
            _ => None,
        }
    }

    fn dia_li_active(&mut self, ch: &mut Channel, idx: usize) {
        if let Some(z) = ch.zone(idx) {
            let n = z.number as i32 + 2000;
            self.dia_wactive(ch, n);
        }
    }

    /// `Dia_LiDraw`: draws the visible lines of a list.
    pub(crate) fn dia_li_draw(&mut self, ch: &mut Channel, it: &mut Interp, idx: usize) {
        let Some(l) = Self::list(ch, idx) else { return };
        let (pos, ty) = (l.pos, l.ty);
        for k in 0..ty.max(0) {
            self.dia_li_aff(ch, it, idx, pos.wrapping_add(k));
        }
    }

    /// `Dia_LiAff`: draws element `n` of a list if visible.
    fn dia_li_aff(&mut self, ch: &mut Channel, it: &mut Interp, idx: usize, n: i16) {
        let Some(l) = Self::list(ch, idx).cloned() else {
            return;
        };
        let row = n.wrapping_sub(l.pos);
        if row < 0 || row >= l.ty {
            return;
        }
        self.dia_locate(ch, 0, row as i32);
        let inv = if n == l.act { b'1' } else { b'0' };
        self.dia_print(ch, &[27, b'I', inv, 26]);
        let Some(a) = l.array else { return };
        let count = self.dia_array_size(it, a).unwrap_or(0);
        if n as u16 as i32 >= count {
            return;
        }
        let mut used = 0;
        if l.lflags & 1 != 0 {
            let v = n as i32 + (l.lflags >> 1 & 1) as i32;
            let digits = dec(v);
            used = digits.len() as i32 + 5;
            let pad = l.larray as i32 - digits.len() as i32;
            let mut out = vec![b' '; pad.max(0) as usize];
            out.extend_from_slice(&digits);
            out.extend_from_slice(b" - ");
            self.dia_print(ch, &out);
        }
        let s = match self.dia_array_get(it, a, n as i32) {
            Some(DVal::Str(s)) => s.to_vec(),
            _ => Vec::new(),
        };
        let max = (l.tx as i32 - used).max(0) as usize;
        let t = &s[..s.len().min(max)];
        self.dia_print(ch, t);
    }

    // ------------------------------------------------------------------
    // Hypertext
    // ------------------------------------------------------------------

    fn text_zone(ch: &mut Channel, idx: usize) -> Option<&mut TextZone> {
        match ch.zone_mut(idx).map(|z| &mut z.kind) {
            Some(ZoneKind::Text(t)) => Some(t),
            _ => None,
        }
    }

    fn dia_tx_active(&mut self, ch: &mut Channel, idx: usize) {
        if let Some(z) = ch.zone(idx) {
            let n = z.number as i32 + 3000;
            self.dia_wactive(ch, n);
        }
    }

    /// `Dia_TxDraw`: draws the visible lines.
    pub(crate) fn dia_tx_draw(&mut self, ch: &mut Channel, idx: usize) {
        let Some(t) = Self::text_zone(ch, idx) else {
            return;
        };
        let (pos, ty) = (t.pos, t.ty);
        for k in 0..ty.max(0) {
            self.dia_tx_aff(ch, idx, pos.wrapping_add(k));
        }
        if let Some(t) = Self::text_zone(ch, idx) {
            t.act = None;
        }
    }

    /// `Dia_TxAff`: draws line `n` of a hypertext zone, recording its
    /// active words: `{[keyword,paper,pen]text}`.
    fn dia_tx_aff(&mut self, ch: &mut Channel, idx: usize, n: i16) {
        let Some(t) = Self::text_zone(ch, idx) else {
            return;
        };
        let row = n.wrapping_sub(t.pos);
        if row < 0 || row >= t.ty {
            return;
        }
        let r = row as usize;
        t.rows[r] = HtRow::default();
        let (tx, pp, nlines, disp_max) = (
            t.tx as i32,
            t.pp.clone(),
            t.lines.len(),
            t.disp_max as usize,
        );
        let (pen, paper) = (t.pen, t.paper);
        let line_info = t.lines.get(n as u16 as usize).copied();
        let text = t.text.clone();
        self.dia_locate(ch, 0, row as i32);
        self.dia_print(ch, &pp);
        self.dia_print(ch, &[26]);
        if n as u16 as usize >= nlines {
            return;
        }
        let Some((ls, len)) = line_info else { return };
        let mut line: Vec<u8> = text.get(ls..ls + len as usize).unwrap_or(&[]).to_vec();
        line.push(0);
        let mut zones: Vec<HtZone> = Vec::new();
        let at = |p: usize| line.get(p).copied().unwrap_or(0);
        let mut p = 0usize;
        let mut scan = 0usize;
        loop {
            let mut q = scan;
            while at(q) != 0 && at(q) != b'{' {
                q += 1;
            }
            let col = self.dia_print3(ch, &line[p..q], tx);
            if col >= tx || at(q) != b'{' {
                break;
            }
            // An active segment.
            let mut ok = zones.len() < disp_max && at(q + 1) == b'[';
            let mut z = HtZone {
                paper: pen,
                pen: paper,
                ..Default::default()
            };
            let mut a1 = q + 2;
            let mut has_kw = false;
            if ok {
                z.c0 = self.dia_cursor(ch).0 as u8;
                z.kw = a1 as u8;
                has_kw = at(a1) != b',';
                loop {
                    let c = at(a1);
                    a1 += 1;
                    match c {
                        0 => {
                            ok = false;
                            break;
                        }
                        b',' => {
                            let num = |a1: &mut usize| {
                                let mut v: u32 = 0;
                                loop {
                                    let c = at(*a1);
                                    *a1 += 1;
                                    if !c.is_ascii_digit() {
                                        return (v, c);
                                    }
                                    v = v * 10 + (c - b'0') as u32;
                                }
                            };
                            let (pa, c) = num(&mut a1);
                            z.paper = b'0'.wrapping_add(pa as u8);
                            if c != b',' {
                                ok = false;
                                break;
                            }
                            let (pe, c) = num(&mut a1);
                            z.pen = b'0'.wrapping_add(pe as u8);
                            if c != b']' {
                                ok = false;
                            }
                            break;
                        }
                        b']' => break,
                        _ => {}
                    }
                }
            }
            if !ok {
                // Not an active word: printed as normal text.
                p = q;
                scan = q + 1;
                continue;
            }
            let start = a1;
            let mut end = a1;
            loop {
                let c = at(end);
                end += 1;
                if c == 0 {
                    break;
                }
                if c == b'}' {
                    end -= 1;
                    break;
                }
            }
            z.start = start as u8;
            z.end = end as u8;
            self.dia_print(ch, &[27, b'B', z.paper, 27, b'P', z.pen]);
            let col = self.dia_print3(ch, &line[start.min(line.len())..end.min(line.len())], tx);
            z.c1 = col as u8;
            if !has_kw {
                z.c0 = 1;
                z.c1 = 0;
            }
            zones.push(z);
            if col >= tx || at(end) != b'}' {
                break;
            }
            p = end + 1;
            scan = p;
            self.dia_print(ch, &pp);
        }
        if let Some(t) = Self::text_zone(ch, idx) {
            t.rows[r] = HtRow {
                line: Some(n as u16 as usize),
                zones,
            };
        }
    }

    /// `Dia_TxAffActive`: draws an active word, highlighted if it is the
    /// current one.
    fn dia_tx_aff_active(&mut self, ch: &mut Channel, idx: usize, row: usize, k: usize) {
        let Some(t) = Self::text_zone(ch, idx) else {
            return;
        };
        let Some(r) = t.rows.get(row) else { return };
        let (Some(line), Some(&z)) = (r.line, r.zones.get(k)) else {
            return;
        };
        let hl = t.act == Some((row, k));
        let (ls, len) = t.lines[line];
        let tx = t.tx as i32;
        let mut s: Vec<u8> = t.text.get(ls..ls + len as usize).unwrap_or(&[]).to_vec();
        s.push(0);
        let txt = s
            .get(z.start as usize..(z.end as usize).min(s.len()))
            .unwrap_or(&[])
            .to_vec();
        if !self.dia_locate(ch, z.c0 as i32, row as i32) {
            return;
        }
        let (b, p) = if hl {
            (z.pen, z.paper)
        } else {
            (z.paper, z.pen)
        };
        self.dia_print(ch, &[27, b'B', b, 27, b'P', p]);
        self.dia_print3(ch, &txt, tx);
    }

    // ------------------------------------------------------------------
    // Sliders
    // ------------------------------------------------------------------

    /// `Dia_SlDraw`: draws a slider then restores the writing mode.
    pub(crate) fn dia_sl_draw(&mut self, ch: &mut Channel, idx: usize, active: bool) {
        let Some(Zone {
            kind: ZoneKind::Slider(sl),
            ..
        }) = ch.zone_mut(idx)
        else {
            return;
        };
        let mut sl = sl.clone();
        let id = ch.screen_id;
        let mut inks = self
            .dialogs
            .slider_inks
            .get(&id)
            .copied()
            .unwrap_or_default();
        let sprites = self.banks.get(1).cloned();
        if let Some(s) = self.screens.get_mut(ch.screen) {
            sl.draw(s, &mut inks, active, sprites.as_ref());
            s.gr.writing = ch.writing;
        }
        self.dialogs.slider_inks.insert(id, inks);
        if let Some(Zone {
            kind: ZoneKind::Slider(z),
            ..
        }) = ch.zone_mut(idx)
        {
            *z = sl;
        }
    }

    fn slider_mut(ch: &mut Channel, idx: usize) -> Option<&mut slider::Slider> {
        match ch.zone_mut(idx).map(|z| &mut z.kind) {
            Some(ZoneKind::Slider(s)) => Some(s),
            _ => None,
        }
    }

    /// Mouse coordinate along the slider (`Sl_XYMouEc`).
    fn dia_slider_mouse(&self, ch: &Channel, vertical: bool) -> i32 {
        let Some(s) = self.screens.get(ch.screen) else {
            return 0;
        };
        if vertical {
            s.y_screen(self.input.mouse_y)
        } else {
            s.x_screen(self.input.mouse_x)
        }
    }

    /// `Dia_SlChange`: the slider moved, calls the change routine.
    fn dia_sl_change(&mut self, ch: &mut Channel, it: &mut Interp, idx: usize) -> DR<()> {
        let p = Self::slider_mut(ch, idx).map_or(0, |s| s.position as u16 as i32);
        if let Some(z) = ch.zone_mut(idx) {
            z.pos = p;
        }
        self.dia_zo_change(ch, it, idx).map(|_| ())
    }

    /// One iteration of the knob drag of `Sl_Clic`.
    fn dia_sl_drag(&mut self, ch: &mut Channel, it: &mut Interp, idx: usize, grab: i32) -> DR<()> {
        let Some(sl) = Self::slider_mut(ch, idx).cloned() else {
            return Ok(());
        };
        let mut d2 = self.dia_slider_mouse(ch, sl.vertical) - grab;
        if d2 < 0 {
            d2 = 0;
        }
        d2 -= sl.start;
        if d2 < 0 {
            d2 = 0;
        }
        let num = (d2 as u16 as u32) * ((sl.global + 1) as u16 as u32);
        let den = (sl.size + 1) as u16 as u32;
        let mut v = if den != 0 && num / den <= 0xFFFF {
            (num / den) as i32
        } else {
            d2
        };
        let max = (sl.global - sl.window).max(0);
        if v as i16 > max as i16 {
            v = max;
        }
        if v != sl.position {
            if let Some(s) = Self::slider_mut(ch, idx) {
                s.position = v;
            }
            self.dia_sl_draw(ch, idx, true);
            if self.input.mouse_buttons & 2 != 0 {
                self.dia_sl_change(ch, it, idx)?;
            }
        }
        Ok(())
    }

    /// Slider arrows: one step down (towards 0) or up.
    fn dia_sl_step(&mut self, ch: &mut Channel, it: &mut Interp, idx: usize, up: bool) -> DR<()> {
        self.dia_sl_draw(ch, idx, true);
        let Some(sl) = Self::slider_mut(ch, idx).cloned() else {
            return Ok(());
        };
        let d0 = if up {
            let mut d0 = sl.position + sl.scroll;
            if ((d0 + sl.window) as u16) > sl.global as u16 {
                d0 = (sl.global - sl.window).max(0);
            }
            d0
        } else {
            (sl.position - sl.scroll).max(0)
        };
        if d0 != sl.position {
            if let Some(s) = Self::slider_mut(ch, idx) {
                s.position = d0;
            }
            self.dia_sl_change(ch, it, idx)?;
        }
        Ok(())
    }

    /// `Sl_Clic`: a click on a slider; returns the modal state.
    fn dia_sl_clic(&mut self, ch: &mut Channel, it: &mut Interp, idx: usize) -> DR<Modal> {
        let Some(sl) = Self::slider_mut(ch, idx).cloned() else {
            return Ok(Modal::Hold {
                zone: idx,
                quit_check: false,
            });
        };
        let d0 = self.dia_slider_mouse(ch, sl.vertical) - sl.start;
        if (d0 as u16) < sl.mouse1 as u16 {
            self.dia_sl_step(ch, it, idx, false)?;
            Ok(Modal::SliderDown { zone: idx })
        } else if (d0 as u16) >= sl.mouse2 as u16 {
            self.dia_sl_step(ch, it, idx, true)?;
            Ok(Modal::SliderUp { zone: idx })
        } else {
            let grab = d0 - sl.mouse1;
            self.dia_sl_draw(ch, idx, true);
            self.dia_sl_drag(ch, it, idx, grab)?;
            Ok(Modal::SliderDrag { zone: idx, grab })
        }
    }

    // ------------------------------------------------------------------
    // The tests
    // ------------------------------------------------------------------

    /// `Dia_GetZ`: the zone under the mouse, with the coordinates inside.
    fn dia_mouse_zone(&self, ch: &Channel) -> Option<(usize, i32, i32)> {
        let s = self.screens.get(ch.screen)?;
        let x = s.x_screen(self.input.mouse_x);
        let y = s.y_screen(self.input.mouse_y);
        Self::dia_zone_at(ch, x, y)
    }

    pub(crate) fn dia_zone_at(ch: &Channel, x: i32, y: i32) -> Option<(usize, i32, i32)> {
        let (x, y) = (x as u16, y as u16);
        for (i, r) in ch.records.iter().enumerate() {
            let Record::Zone(z) = r else { continue };
            let (zx, zy) = (z.x as u16, z.y as u16);
            if x >= zx
                && x < zx.wrapping_add(z.sx as u16)
                && y >= zy
                && y < zy.wrapping_add(z.sy as u16)
            {
                return Some((i, (x - zx) as i32, (y - zy) as i32));
            }
        }
        None
    }

    /// `Dia_Tests`: one pass of the user interaction. Returns the number of
    /// exit requests.
    pub(crate) fn dia_tests(&mut self, it: &mut Interp, ch: &mut Channel) -> DR<i16> {
        ch.exit = 0;
        self.dia_update_last_key();
        if ch.modal.is_some() {
            self.dia_modal(it, ch)?;
            return Ok(ch.exit);
        }
        // A button pressed with NoWait: repeats while held.
        if let Some(z) = ch.release {
            ch.ret = ch.zone(z).map_or(0, |z| z.number);
            if self.input.mouse_buttons & 1 != 0 {
                self.dia_zo_change(ch, it, z)?;
                return Ok(ch.exit);
            }
            ch.ret = 0;
            ch.release = None;
            self.dia_bt_draw(ch, it, z)?;
        }
        let key = match ch.edited {
            Edited::Zone(z) => {
                self.dia_timer(ch);
                self.dia_ed_active(ch, z);
                let (k, code) = self.led_loop(ch, z);
                if code == 1 {
                    ch.ret = ch.zone(z).map_or(0, |z| z.number);
                    if ch.flags & 0x10 == 0 {
                        self.dia_ed_next(ch, z);
                    }
                    return self.dia_tests_mouse(it, ch);
                }
                match k {
                    None => return self.dia_tests_mouse(it, ch),
                    Some(k) if k.raw == 0x42 => {
                        self.dia_ed_next(ch, z);
                        return self.dia_tests_mouse(it, ch);
                    }
                    Some(k) => Some(k),
                }
            }
            _ => self.dialogs.cl_last,
        };
        ch.last_key = key;
        let Some(k) = key else {
            return self.dia_tests_mouse(it, ch);
        };
        self.dia_timer(ch);
        if ch.flags & 4 != 0 {
            ch.exit += 1;
        }
        let ascii = k.ascii.to_ascii_uppercase();
        for i in 0..ch.records.len() {
            let Record::Key { code, shift, zone } = ch.records[i] else {
                continue;
            };
            let hit = match code {
                0 => false,
                0xFF => true,
                c if c >= 0x80 => (c & 0x7F) == k.raw,
                c => c == ascii,
            };
            if !hit {
                continue;
            }
            let same = |g: u8| (shift & g != 0) == (k.shift & g != 0);
            if !(same(SHF) && same(AMI) && same(CTR) && same(ALT)) {
                continue;
            }
            let Some(z) = zone else {
                return self.dia_tests_mouse(it, ch);
            };
            self.dia_clear_key();
            self.dia_tests_zones(it, ch, true, Some((z, 0, 0)))?;
            return Ok(ch.exit);
        }
        // No shortcut: Tab goes back to the first edit zone.
        if ch.edited == Edited::Off && k.raw == 0x42 {
            self.dia_ed_first(ch);
            self.dia_clear_key();
            return Ok(ch.exit);
        }
        self.dia_tests_mouse(it, ch)
    }

    /// `.EdNxt`: the next edit zone gets the keys.
    fn dia_ed_next(&mut self, ch: &mut Channel, z: usize) {
        self.dia_ed_inactive(ch, z);
        ch.edited = Edited::Off;
        self.dia_ed_from(ch, z + 1);
        self.dia_clear_key();
    }

    fn dia_tests_mouse(&mut self, it: &mut Interp, ch: &mut Channel) -> DR<i16> {
        let down = self.input.mouse_buttons & 1 != 0;
        let mz = self.dia_mouse_zone(ch);
        self.dia_tests_zones(it, ch, down, mz)?;
        Ok(ch.exit)
    }

    /// `.MousIn`: the zones and the mouse (or a simulated click).
    fn dia_tests_zones(
        &mut self,
        it: &mut Interp,
        ch: &mut Channel,
        down: bool,
        mz: Option<(usize, i32, i32)>,
    ) -> DR<()> {
        let (d5, d6, d7) = match mz {
            Some((z, x, y)) => (Some(z), x, y),
            None => (None, 0, 0),
        };
        for i in 0..ch.records.len() {
            let Some(z) = ch.zone(i) else { continue };
            let here = d5 == Some(i);
            match &z.kind {
                ZoneKind::Button { min, max, .. } => {
                    if !(down && here) {
                        continue;
                    }
                    let (min, max) = (*min, *max);
                    let number = z.number;
                    let mut p = z.pos.wrapping_add(1);
                    if p as i16 > max {
                        p = min as i32;
                    }
                    ch.zone_mut(i).unwrap().pos = p;
                    self.dia_bt_draw(ch, it, i)?;
                    let d0 = int(&self.dia_zo_change(ch, it, i)?).unwrap_or(p);
                    let z = ch.zone_mut(i).unwrap();
                    if z.flags & 0x20 != 0 {
                        z.flags &= !0x20;
                        z.pos = d0;
                        ch.release = Some(i);
                        ch.ret = number;
                        self.dia_quit_check(ch, i);
                    } else {
                        ch.modal = Some(Modal::Button { zone: i, pos: d0 });
                        return Ok(());
                    }
                }
                ZoneKind::Edit { .. } => {
                    if !(down && here) {
                        continue;
                    }
                    let prev = ch.edited;
                    ch.edited = Edited::Zone(i);
                    if let Edited::Zone(p) = prev
                        && p != i
                    {
                        self.dia_ed_inactive(ch, p);
                    }
                }
                ZoneKind::Slider(_) => {
                    if !(down && here) {
                        continue;
                    }
                    let m = self.dia_sl_clic(ch, it, i)?;
                    ch.modal = Some(m);
                    return Ok(());
                }
                ZoneKind::List(l) => {
                    let l = l.clone();
                    let d2: i16 = if here {
                        let r = (d7 >> 3) as i16 + l.pos;
                        if r < l.max_act { r } else { -1 }
                    } else {
                        -1
                    };
                    let click_only = l.lflags & 4 != 0;
                    let mut set_new = true;
                    if l.act >= 0 {
                        if l.act == d2 || (click_only && (!down || d2 < 0)) {
                            set_new = false;
                        } else {
                            let old = l.act;
                            if let Some(l) = Self::list_mut(ch, i) {
                                l.act = -1;
                            }
                            ch.zone_mut(i).unwrap().pos = -1;
                            self.dia_li_active(ch, i);
                            self.dia_li_aff(ch, it, i, old);
                        }
                    }
                    if set_new && !(click_only && !down) && d2 >= 0 {
                        if let Some(l) = Self::list_mut(ch, i) {
                            l.act = d2;
                        }
                        self.dia_li_active(ch, i);
                        self.dia_li_aff(ch, it, i, d2);
                    }
                    if down && d2 >= 0 {
                        ch.zone_mut(i).unwrap().pos = d2 as i32;
                        self.dia_zo_change(ch, it, i)?;
                        let z = ch.zone_mut(i).unwrap();
                        if z.flags & 0x20 != 0 {
                            z.flags &= !0x20;
                            ch.ret = z.number;
                            self.dia_quit_check(ch, i);
                        } else {
                            ch.modal = Some(Modal::Hold {
                                zone: i,
                                quit_check: true,
                            });
                            return Ok(());
                        }
                    }
                }
                ZoneKind::Text(_) => {
                    // The active word under the mouse.
                    let mut found: Option<(usize, usize)> = None;
                    if here {
                        let (row, col) = ((d7 >> 3) as usize, d6 >> 3);
                        if let Some(t) = Self::text_zone(ch, i)
                            && let Some(r) = t.rows.get(row)
                            && r.line.is_some()
                        {
                            for (k, hz) in r.zones.iter().enumerate() {
                                if col >= hz.c0 as i32 && col < hz.c1 as i32 {
                                    found = Some((row, k));
                                    break;
                                }
                            }
                        }
                    }
                    let act = Self::text_zone(ch, i).and_then(|t| t.act);
                    if act != found {
                        if let Some((r, k)) = act {
                            ch.zone_mut(i).unwrap().pos = -1;
                            self.dia_tx_active(ch, i);
                            if let Some(t) = Self::text_zone(ch, i) {
                                t.act = None;
                            }
                            self.dia_tx_aff_active(ch, i, r, k);
                        }
                        if let Some(t) = Self::text_zone(ch, i) {
                            t.act = found;
                        }
                        if let Some((r, k)) = found {
                            self.dia_tx_active(ch, i);
                            self.dia_tx_aff_active(ch, i, r, k);
                        }
                    }
                    if down && let Some((r, k)) = found {
                        self.dia_tx_click(ch, i, r, k);
                        self.dia_zo_change(ch, it, i)?;
                        let z = ch.zone_mut(i).unwrap();
                        if z.flags & 0x20 != 0 {
                            z.flags &= !0x20;
                            ch.ret = z.number;
                        } else {
                            ch.modal = Some(Modal::Hold {
                                zone: i,
                                quit_check: false,
                            });
                            return Ok(());
                        }
                    }
                }
            }
        }
        self.dia_mouse_exit(ch, down);
        Ok(())
    }

    /// After the zones: a click resets the timer and may exit (RUn flag 3).
    fn dia_mouse_exit(&mut self, ch: &mut Channel, down: bool) {
        if down {
            self.dia_timer(ch);
            if ch.flags & 8 != 0 {
                ch.exit += 1;
            }
        }
    }

    /// `.BQuit`: the zone asked to quit (`BQ`).
    fn dia_quit_check(&mut self, ch: &mut Channel, i: usize) {
        if let Some(z) = ch.zone_mut(i)
            && z.flags & 0x80 != 0
        {
            z.flags &= !0x80;
            ch.exit += 1;
        }
    }

    /// Hypertext click: the keyword is a line number or a text.
    fn dia_tx_click(&mut self, ch: &mut Channel, idx: usize, row: usize, k: usize) {
        let Some(t) = Self::text_zone(ch, idx) else {
            return;
        };
        let Some(line) = t.rows.get(row).and_then(|r| r.line) else {
            return;
        };
        let Some(hz) = t.rows[row].zones.get(k).copied() else {
            return;
        };
        let (ls, len) = t.lines[line];
        let mut s: Vec<u8> = t.text.get(ls..ls + len as usize).unwrap_or(&[]).to_vec();
        s.push(0);
        let mut pc = hz.kw as usize;
        let (c, cl) = lexer::chr(&s, &mut pc);
        let pos;
        if cl == lexer::Class::Num {
            pos = lexer::number(&s, &mut pc, c);
            t.buffer.clear();
        } else {
            let mut b = Vec::new();
            let mut p = pc - 1;
            while b.len() < 62 {
                let c = s.get(p).copied().unwrap_or(0);
                p += 1;
                if c == b',' || c == b']' || c == 0 {
                    break;
                }
                b.push(c);
            }
            t.buffer = b;
            pos = 0;
        }
        if let Some(z) = ch.zone_mut(idx) {
            z.pos = pos;
        }
    }

    /// Continues a mouse hold.
    fn dia_modal(&mut self, it: &mut Interp, ch: &mut Channel) -> DR<()> {
        let left = self.input.mouse_buttons & 1 != 0;
        let right = self.input.mouse_buttons & 2 != 0;
        let Some(m) = ch.modal.clone() else {
            return Ok(());
        };
        match m {
            Modal::Button { zone, pos } => {
                if left {
                    self.dia_screen_follow();
                    return Ok(());
                }
                self.dialogs.screen_move = None;
                ch.modal = None;
                ch.ret = ch.zone(zone).map_or(0, |z| z.number);
                if ch.zone(zone).is_some_and(|z| z.pos != pos) {
                    ch.zone_mut(zone).unwrap().pos = pos;
                    self.dia_bt_draw(ch, it, zone)?;
                }
                self.dia_quit_check(ch, zone);
                self.dia_mouse_exit(ch, true);
            }
            Modal::Hold { zone, quit_check } => {
                if left {
                    return Ok(());
                }
                ch.modal = None;
                ch.ret = ch.zone(zone).map_or(0, |z| z.number);
                if quit_check {
                    self.dia_quit_check(ch, zone);
                }
                self.dia_mouse_exit(ch, true);
            }
            Modal::SliderDown { zone } | Modal::SliderUp { zone } => {
                if left {
                    if right {
                        self.dia_sl_step(ch, it, zone, matches!(m, Modal::SliderUp { .. }))?;
                    }
                    return Ok(());
                }
                self.dia_slider_end(ch, zone);
            }
            Modal::SliderDrag { zone, grab } => {
                if left {
                    return self.dia_sl_drag(ch, it, zone, grab);
                }
                self.dia_sl_change(ch, it, zone)?;
                self.dia_slider_end(ch, zone);
            }
        }
        Ok(())
    }

    fn dia_slider_end(&mut self, ch: &mut Channel, zone: usize) {
        ch.modal = None;
        self.dia_sl_draw(ch, zone, false);
        ch.ret = ch.zone(zone).map_or(0, |z| z.number);
        if let Some(z) = ch.zone_mut(zone) {
            z.flags &= !0x80;
        }
        self.dia_mouse_exit(ch, true);
    }

    /// `SM`: the screen follows the mouse vertically while the button is
    /// held.
    fn dia_screen_follow(&mut self) {
        let Some((n, dy)) = self.dialogs.screen_move else {
            return;
        };
        let y = self.input.mouse_y - dy;
        if let Some(s) = self.screens.get_mut(n)
            && s.display_y != y
        {
            s.pending_display[1] = Some(y);
        }
    }

    // ------------------------------------------------------------------
    // Values, updates
    // ------------------------------------------------------------------

    /// `Dia_GetValue`: value of zone `n` (k-th of that number): a number,
    /// or a string for edit and hypertext zones.
    pub(crate) fn dia_get_value(&mut self, n: i64, zone: i32, k: i32) -> DR<DVal> {
        let ch = self
            .dialogs
            .channels
            .iter_mut()
            .find(|c| c.number == n)
            .ok_or(e::CHANNEL_NOT_DEFINED)?;
        let i = ch.find_zone(zone, k).ok_or(e::CHANNEL_NOT_DEFINED)?;
        let z = ch.zone_mut(i).unwrap();
        Ok(match &mut z.kind {
            ZoneKind::Button { .. } | ZoneKind::List(_) | ZoneKind::Slider(_) => DVal::Int(z.pos),
            ZoneKind::Text(t) => {
                if z.pos != 0 {
                    DVal::Int(z.pos)
                } else {
                    DVal::str(&t.buffer)
                }
            }
            ZoneKind::Edit {
                led,
                digit: Some(v),
            } => {
                if let Some(&c) = led.buf.first()
                    && c >= 32
                {
                    let mut b = led.buf.clone();
                    b.push(0);
                    let mut pc = 1;
                    *v = lexer::number(&b, &mut pc, c);
                }
                DVal::Int(*v)
            }
            ZoneKind::Edit { led, digit: None } => DVal::str(&led.buf),
        })
    }

    /// `Dia_ZUpdate`: changes the zones numbered `n` (but the zone being
    /// drawn / changed). None = parameter omitted.
    pub(crate) fn dia_zupdate(
        &mut self,
        ch: &mut Channel,
        it: &mut Interp,
        n: i32,
        p1: Option<DVal>,
        p2: Option<i32>,
        p3: Option<i32>,
    ) -> DR<()> {
        for i in 0..ch.records.len() {
            let Some(z) = ch.zone(i) else { continue };
            if z.number != n as i16 || ch.v_zone == Some(i) {
                continue;
            }
            match &z.kind {
                ZoneKind::Button { .. } => {
                    let new = match &p1 {
                        None => None,
                        Some(v) => Some(int(v)?),
                    };
                    let changed = match new {
                        None => true,
                        Some(p) if p != z.pos => {
                            ch.zone_mut(i).unwrap().pos = p;
                            true
                        }
                        _ => false,
                    };
                    if changed {
                        self.dia_bt_draw(ch, it, i)?;
                        self.dia_zo_change(ch, it, i)?;
                    }
                    ch.zone_mut(i).unwrap().flags &= !0x80;
                }
                ZoneKind::Slider(_) => {
                    let p1 = match &p1 {
                        None => None,
                        Some(v) => Some(int(v)?),
                    };
                    let sl = Self::slider_mut(ch, i).unwrap();
                    if let Some(g) = p3 {
                        sl.global = g as u16 as i32;
                    }
                    if let Some(w) = p2 {
                        sl.window = w as u16 as i32;
                    }
                    if let Some(p) = p1 {
                        sl.position = p as u16 as i32;
                        ch.zone_mut(i).unwrap().pos = p;
                    }
                    self.dia_sl_draw(ch, i, false);
                    self.dia_zo_change(ch, it, i)?;
                    ch.zone_mut(i).unwrap().flags &= !0x80;
                }
                ZoneKind::List(_) => {
                    let p1 = match &p1 {
                        None => None,
                        Some(v) => Some(int(v)?),
                    };
                    if let Some(m) = p3 {
                        Self::list_mut(ch, i).unwrap().max_act = m as i16;
                    }
                    if let Some(a) = p2 {
                        let old = Self::list(ch, i).unwrap().act;
                        if old >= 0 {
                            Self::list_mut(ch, i).unwrap().act = -1;
                            self.dia_li_active(ch, i);
                            self.dia_li_aff(ch, it, i, old);
                        }
                        Self::list_mut(ch, i).unwrap().act = a as i16;
                        ch.zone_mut(i).unwrap().pos = a;
                    }
                    if let Some(p) = p1 {
                        Self::list_mut(ch, i).unwrap().pos = p as i16;
                    }
                    self.dia_li_active(ch, i);
                    self.dia_li_draw(ch, it, i);
                    ch.zone_mut(i).unwrap().flags &= !0x80;
                }
                ZoneKind::Text(_) => {
                    let Some(v) = &p1 else { continue };
                    let p = int(v)?;
                    Self::text_zone(ch, i).unwrap().pos = p as i16;
                    self.dia_tx_active(ch, i);
                    self.dia_tx_draw(ch, i);
                    ch.zone_mut(i).unwrap().flags &= !0x80;
                }
                ZoneKind::Edit { digit, .. } => {
                    let Some(v) = &p1 else { continue };
                    let t = match digit {
                        None => match v {
                            DVal::Str(s) => s.to_vec(),
                            _ => return Err(e::FCALL),
                        },
                        Some(_) => dec(int(v)?),
                    };
                    let t: Vec<u8> = t.iter().copied().take(255).collect();
                    self.dia_ed_on(ch, i);
                    self.led_new(ch, i, &t);
                }
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Erasing
    // ------------------------------------------------------------------

    /// `Dia_EffChannel`: erases the dialog of channel `n`.
    pub(crate) fn dia_eff_channel(&mut self, it: &mut Interp, n: i64) -> DR<()> {
        let (i, mut ch) = self.dialogs.take(n).ok_or(e::CHANNEL_NOT_DEFINED)?;
        self.dia_eff_chan(it, &mut ch);
        self.dialogs.put(i, ch);
        Ok(())
    }

    /// `Dia_EffChanA0`: closes the windows of the zones and restores the
    /// saved backgrounds (in reverse order).
    pub(crate) fn dia_eff_chan(&mut self, it: &mut Interp, ch: &mut Channel) {
        let _ = it;
        if ch.rflags & 1 == 0 {
            return;
        }
        ch.rflags &= !1;
        if self.dia_active(ch).is_err() {
            return;
        }
        ch.modal = None;
        let mut blocks = Vec::new();
        for i in 0..ch.records.len() {
            let wn = match &ch.records[i] {
                Record::Block(b) => {
                    blocks.push(*b);
                    continue;
                }
                Record::Zone(z) => match z.kind {
                    ZoneKind::Edit { .. } => z.number as i32 + 1000,
                    ZoneKind::List(_) => z.number as i32 + 2000,
                    ZoneKind::Text(_) => z.number as i32 + 3000,
                    _ => continue,
                },
                _ => continue,
            };
            self.dia_wactive(ch, wn);
            if let Some(s) = self.screens.get_mut(ch.screen) {
                let _ = s.print_text(b"\x1bJ0");
                let _ = s.wind_close();
            }
            ch.wind_on = -1;
        }
        for &b in blocks.iter().rev() {
            if let Some(blk) = self.draw.blocks.iter().find(|o| o.number == b).cloned()
                && let Some(s) = self.screens.get_mut(ch.screen)
            {
                let (w, h, sp, clip) = (s.width as i32, s.height as i32, s.planes, s.gr.clip);
                crate::gfx::blocks::put_block(
                    &blk,
                    s.logic_mut(),
                    w,
                    h,
                    sp,
                    clip,
                    blk.x,
                    blk.y,
                    0xFFFF,
                    None,
                );
            }
            self.draw.blocks.retain(|o| o.number != b);
        }
        ch.edited = Edited::None;
        self.dia_reactive(ch);
    }

    // ------------------------------------------------------------------
    // Automatic tests (`Dia_AutoTest` `+Lib.s:24081`)
    // ------------------------------------------------------------------

    /// Tests the displayed, non frozen channels; called once per frame by
    /// the interpreter's test point. A channel that exits is erased.
    pub(crate) fn dia_auto_test(&mut self, it: &mut Interp, max: usize) -> DR<()> {
        let numbers: Vec<i64> = self
            .dialogs
            .channels
            .iter()
            .map(|c| c.number)
            .take(max)
            .collect();
        for n in numbers {
            let Some((i, mut ch)) = self.dialogs.take(n) else {
                continue;
            };
            let r = self.dia_auto_test_one(it, &mut ch);
            if r.is_err() {
                ch.rflags |= 4;
            }
            self.dialogs.put(i, ch);
            r?;
        }
        Ok(())
    }

    fn dia_auto_test_one(&mut self, it: &mut Interp, ch: &mut Channel) -> DR<()> {
        if ch.rflags & 1 == 0 || ch.rflags & 4 != 0 || ch.run.is_some() {
            return Ok(());
        }
        self.dia_active(ch)?;
        if let Edited::Zone(z) = ch.edited {
            self.dia_ed_active(ch, z);
        }
        ch.error = 0;
        let r = self.dia_tests(it, ch);
        match r {
            Ok(x) if x != 0 => self.dia_eff_chan(it, ch),
            _ => {}
        }
        self.dia_reactive(ch);
        r.map(|_| ())
    }
}
