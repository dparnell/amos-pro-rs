//! Menu instructions (`+ILib.s:6731-6950`, `+Lib.s:15326-15760`) and the
//! menu bar itself (`MnGere` and the drawing routines, `+Lib.s:15782-17020`).
//!
//! The menu is drawn directly into the screen that was current when it was
//! defined (`MnAdEc`), saving the background under each branch. While the
//! right mouse button is held the program is stopped, as in the original
//! where `MnGere` loops on `WaitVbl` inside the interpreter's tests: here the
//! loop runs one iteration per VBL from the test point and the instruction
//! that reached the test point waits.

use std::collections::HashMap;

use super::Hardware;
use crate::banks::Image;
use crate::errors;
use crate::gfx::draw::{COMPLEMENT, Canvas, GrState, JAM2, builtin_pattern};
use crate::gfx::{gfont, images};
use crate::interp::value::Value;
use crate::interp::{Ctl, Exc, Interp, R, WaitKind, err};
use crate::menus::{self, MenuItem, MenuKey, MenuOp, OnMenu, OnMenuKind, flags};
use crate::tokens::{Keyword, TK_COMMA, TK_PAR1, TK_PAR2, TK_TO, tk};

/// "Menu not opened".
const MENU_NOT_OPENED: u16 = 38;
/// "Menu item not defined".
const ITEM_NOT_DEFINED: u16 = 39;

/// Saved background of a branch (`MnSave`).
struct Saved {
    x: i32,
    y: i32,
    w: usize,
    h: usize,
    pixels: Vec<u8>,
}

/// Zone rectangle (inclusive).
type ZoneRect = (i32, i32, i32, i32);

/// An item's zone (`SetZone` in `MnBranch`).
struct Zone {
    level: usize,
    index: usize,
    path: Vec<u16>,
    rect: ZoneRect,
}

/// Moving a menu or an item with the left button (`MnBGoch`).
struct Drag {
    /// The clicked item and the first item of the branch to redraw.
    item: Vec<u16>,
    head: Vec<u16>,
    /// Item position minus mouse position.
    off: (i32, i32),
    /// Limits of the outline (left, top, right, bottom).
    lim: (i32, i32, i32, i32),
    size: (i32, i32),
    /// Outline currently drawn (complement mode) and its drawing state.
    shown: Option<((i32, i32, i32, i32), GrState)>,
    line: u16,
    pos: (i32, i32),
}

/// The open menu (`MnGere` state: `MnTable`, `MnAct`, `MnZoAct`...).
pub struct MenuSession {
    screen: usize,
    /// Items whose sub-menu is open, one per level (`MnTable`).
    table: Vec<Vec<u16>>,
    /// Highlighted item (`MnAct` when `MnZoAct` is not 0).
    act: Option<Vec<u16>>,
    zones: Vec<Zone>,
    saves: HashMap<Vec<u16>, Saved>,
    /// Position of the menu bar (`MnBaseX`, `MnBaseY`).
    base: (i32, i32),
    /// Drawing state of the menu (the RastPort while the menu is open).
    gr: GrState,
    drag: Option<Drag>,
    amal_frozen: bool,
}

/// Flags of the item drawing routine (`MnDraw` d7 bits 31/30/29).
#[derive(Clone, Copy)]
struct DrawMode {
    calc: bool,
    active: bool,
    background: bool,
}

/// Images used by the `(BO n)` / `(IC n)` commands.
struct ImageBanks<'a> {
    bobs: Option<&'a Vec<Image>>,
    icons: Option<&'a Vec<Image>>,
}

impl ImageBanks<'_> {
    /// Image `n` (`L_AdBob` / `L_AdIcon`), none if missing or empty.
    fn get(&self, icons: bool, n: i16) -> Option<&Image> {
        let bank = if icons { self.icons } else { self.bobs }?;
        let img = bank.get((n as i32 - 1).max(0) as usize).filter(|_| n > 0)?;
        if img.is_empty() { None } else { Some(img) }
    }
}

/// Draws (or measures when `c` is None) a compiled object at `org`
/// (`MnODraw`, `+Lib.s:16624`). Returns the cursor and the maximum size
/// reached, relative to `org`.
fn draw_object(
    ops: &[MenuOp],
    inks: (u8, u8, u8),
    org: (i32, i32),
    mode: DrawMode,
    gr: &mut GrState,
    mut c: Option<&mut Canvas>,
    images: &ImageBanks,
) -> ((i32, i32), (i32, i32)) {
    let (ax, ay) = org;
    let (mut x, mut y) = (0i32, 0i32);
    let (mut mx, mut my) = (0i32, 0i32);
    if !mode.calc {
        gr.ink = inks.0;
        gr.paper = inks.1;
        gr.outline = inks.2;
        gr.writing = JAM2;
    }
    for op in ops {
        match op {
            MenuOp::Text(t) => {
                if let Some(c) = c.as_deref_mut() {
                    gr.text(c, x + ax, y + gfont::BASELINE + ay, t);
                }
                x += gfont::WIDTH * t.len() as i32;
                y += gfont::HEIGHT;
            }
            MenuOp::Locate(lx, ly) => {
                if *lx >= 0 && *ly >= 0 {
                    x = *lx as i32;
                    y = *ly as i32;
                }
            }
            MenuOp::Bar(bx, by) => {
                // Unsigned compares: the bar must go right and down.
                let (bx, by) = (*bx as i32, *by as i32);
                if bx >= 0 && by >= 0 && (bx as u16) > (x as u16) && (by as u16) > (y as u16) {
                    if let Some(c) = c.as_deref_mut() {
                        gr.bar(c, x + ax, y + ay, bx + ax, by + ay);
                    }
                    x = bx + 1;
                    y = by + 1;
                }
            }
            MenuOp::Line(lx, ly) => {
                let (lx, ly) = (*lx as i32, *ly as i32);
                if lx >= 0 && ly >= 0 {
                    gr.x = x + ax;
                    gr.y = y + ay;
                    if let Some(c) = c.as_deref_mut() {
                        gr.draw_to(c, lx + ax, ly + ay);
                    }
                    x = lx + 1;
                    y = ly + 1;
                }
            }
            MenuOp::Ellipse(rx, ry) => {
                let (rx, ry) = (*rx as i32, *ry as i32);
                if rx != 0 && ry != 0 {
                    if let Some(c) = c.as_deref_mut() {
                        gr.ellipse(c, x + ax, y + ay, rx, ry);
                    }
                    x += rx + 1;
                    y += ry + 1;
                }
            }
            MenuOp::Pattern(n) => {
                if !mode.calc {
                    gr.pattern_number = *n as i32;
                    gr.pattern = if *n > 0 {
                        builtin_pattern(*n as usize)
                    } else {
                        None
                    };
                }
            }
            MenuOp::Ink(n, col) => {
                if !mode.calc {
                    let col = *col as u8;
                    match n - 2 {
                        ..0 => gr.ink = col,
                        0 => gr.paper = col,
                        _ => gr.outline = col,
                    }
                }
            }
            MenuOp::Bob(n) | MenuOp::Icon(n) => {
                let icons = matches!(op, MenuOp::Icon(_));
                if let Some(img) = images.get(icons, *n) {
                    if let Some(c) = c.as_deref_mut() {
                        paste_image(c, img, x + ax, y + ay, icons);
                    }
                    x += img.width() as i32;
                    y += img.height as i32;
                }
            }
            // The original flips bit 4 of rp_Flags instead of AREAOUTLINE
            // (bit 3, always set while the menu is open): no effect.
            MenuOp::Outline(_) => {}
            MenuOp::SetLine(n) => gr.line_pattern = *n as u16,
            MenuOp::SetFont(n) => gr.font = *n as i32,
            MenuOp::Style(n) => gr.text_style = *n as u8,
            // Procedures drawing items are not supported: nothing drawn.
            MenuOp::Proc(_) | MenuOp::Reserve(_) => {}
        }
        mx = mx.max(x);
        my = my.max(y);
    }
    ((x, y), (mx, my))
}

/// `Paste Bob` / `Paste Icon` of a menu item: colour 0 is transparent for
/// bobs, icons are opaque.
fn paste_image(c: &mut Canvas, img: &Image, x: i32, y: i32, icons: bool) {
    let w = img.width() as i32;
    let px = images::chunky(img, 0);
    for j in 0..img.height as i32 {
        for i in 0..w {
            let v = px[(j * w + i) as usize];
            if v != 0 || icons {
                c.set(x + i, y + j, v);
            }
        }
    }
}

/// Draws or measures an item (`MnDraw`, `+Lib.s:16552`): the background
/// object, then the normal, selected or inactive one. Returns the cursor
/// after the last object and the size of the item.
fn draw_item(
    item: &MenuItem,
    org: (i32, i32),
    mode: DrawMode,
    gr: &mut GrState,
    mut c: Option<&mut Canvas>,
    images: &ImageBanks,
) -> ((i32, i32), (i32, i32)) {
    let [a1, b1, c1, a2, b2, c2] = item.inks;
    let (mut cur, mut size) = ((0, 0), (0, 0));
    let mut org = org;
    let mut mode = mode;
    if let Some(bg) = &item.objects[3]
        && mode.background
    {
        (cur, size) = draw_object(bg, (b1, a1, c1), org, mode, gr, c.as_deref_mut(), images);
        org = (org.0 + cur.0, org.1 + cur.1);
    }
    let inactive = item.flags & flags::INACTIVE != 0 && item.objects[2].is_some();
    let chosen = if inactive {
        mode.active = false;
        Some((item.objects[2].as_ref().unwrap(), (a1, b1, c1)))
    } else if let Some(normal) = &item.objects[0] {
        if !mode.active {
            Some((normal, (a1, b1, c1)))
        } else if let Some(sel) = &item.objects[1] {
            mode.active = false;
            Some((sel, (a1, b1, c1)))
        } else {
            Some((normal, (a2, b2, c2)))
        }
    } else {
        None
    };
    if let Some((ops, inks)) = chosen {
        let (c2, s2) = draw_object(ops, inks, org, mode, gr, c, images);
        cur = c2;
        // Unsigned compares of the sizes.
        size = (
            (size.0 as u16).max(s2.0 as u16) as i16 as i32,
            (size.1 as u16).max(s2.1 as u16) as i16 as i32,
        );
    }
    (cur, size)
}

/// Computes the positions and sizes of a list of items (`MnCa0`,
/// `+Lib.s:16351`). `(x, y)` is the offset of the first item.
fn calc_list(list: &mut [MenuItem], mut x: i32, mut y: i32, gr: &mut GrState, images: &ImageBanks) {
    let mode = DrawMode {
        calc: true,
        active: false,
        background: true,
    };
    for (i, item) in list.iter_mut().enumerate() {
        // A new group without background object gets a 2 pixel border.
        if (i == 0 || item.flags & flags::SEPARATE != 0) && item.objects[3].is_none() {
            x += 2;
            y += 2;
        }
        if item.flags & flags::FIXED == 0 {
            item.x = x;
            item.y = y;
        }
        let (cur, size) = draw_item(item, (0, 0), mode, gr, None, images);
        (item.tx, item.ty) = size;
        let bar = item.flags & flags::BAR != 0;
        if !item.children.is_empty() {
            let (cx, cy) = if bar { (cur.0, 0) } else { (0, cur.1) };
            calc_list(&mut item.children, cx, cy, gr, images);
        }
        (x, y) = if bar { (0, cur.1) } else { (cur.0, 0) };
    }
}

/// Computes the absolute positions of the group of items starting at
/// `start` (up to the next separated item) from `base` (`MnMaxi`,
/// `+Lib.s:16480`). Returns the rectangle of the group (with its border)
/// and the index after the group.
fn maxi(
    list: &mut [MenuItem],
    start: usize,
    base: (i32, i32),
    screen_w: i32,
) -> ((i32, i32, i32, i32), usize) {
    let (mut px, mut py) = base;
    let (mut x1, mut y1, mut x2, mut y2) = (32766, 32766, 0, 0);
    let mut end = start;
    while end < list.len() && (end == start || list[end].flags & flags::SEPARATE == 0) {
        let it = &mut list[end];
        px += it.x;
        py += it.y;
        it.xx = px;
        it.yy = py;
        x1 = x1.min(px);
        y1 = y1.min(py);
        x2 = x2.max(px).max(px + it.tx);
        y2 = y2.max(py).max(py + it.ty);
        end += 1;
    }
    let head = &mut list[start];
    head.mx = x2 - x1;
    head.my = y2 - y1;
    if head.objects[3].is_none() {
        x1 -= 2;
        y1 -= 2;
        x2 += 2;
        y2 += 2;
        if head.flags & flags::BAR == 0 && head.flags & flags::TLINE != 0 {
            x1 = 0;
            x2 = screen_w;
        }
    }
    ((x1, y1, x2, y2), end)
}

impl Hardware {
    pub(crate) fn menus_instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<bool> {
        if kw.slot != 0 {
            return Ok(false);
        }
        use tk::*;
        match kw.token {
            MENU_S => {
                // Menu$(path)=normal$[,selected$[,inactive$[,background$]]]
                let path = self.menu_path(it)?;
                it.expect(OP_EQ)?;
                let mut strings: [Option<Vec<u8>>; 4] = Default::default();
                for (i, slot) in strings.iter_mut().enumerate() {
                    if i > 0 {
                        if it.peek() != TK_COMMA {
                            break;
                        }
                        it.pc += 2;
                    }
                    if it.peek() == TK_COMMA || Interp::is_end(it.peek()) {
                        continue;
                    }
                    *slot = Some(it.eval_str(self)?.to_vec());
                }
                // The menu belongs to the screen current when the first
                // item was defined (+ILib.s:6850).
                let cur = self
                    .screens
                    .current
                    .ok_or(Exc::Error(errors::SCREEN_NOT_OPENED))?;
                if self.menus.screen.is_some_and(|m| m != cur) {
                    return err(errors::SCREEN_NOT_OPENED);
                }
                let (paper, pen) = self
                    .screens
                    .get(cur)
                    .and_then(|s| s.text.windows.first())
                    .map_or((1, 2), |w| (w.paper as u8, w.pen as u8));
                let mut objects: [Option<Option<Vec<menus::MenuOp>>>; 4] = Default::default();
                for (o, s) in objects.iter_mut().zip(&strings) {
                    if let Some(s) = s {
                        *o = Some(if s.is_empty() {
                            None
                        } else {
                            Some(
                                menus::compile(s)
                                    .ok_or(Exc::Error(errors::ILLEGAL_FUNCTION_CALL))?,
                            )
                        });
                    }
                }
                let item = self
                    .menus
                    .item_create(&path)
                    .ok_or(Exc::Error(errors::ILLEGAL_FUNCTION_CALL))?;
                item.inks = [paper, pen, paper, pen, paper, paper];
                for (i, o) in objects.into_iter().enumerate() {
                    if let Some(o) = o {
                        item.objects[i] = o;
                    }
                }
                self.menus.screen = Some(cur);
            }
            MENU_DEL => {
                let path = if it.peek() == TK_PAR1 {
                    self.menu_path(it)?
                } else {
                    Vec::new()
                };
                self.menus.delete(&path);
            }
            SET_MENU => {
                let path = self.menu_path(it)?;
                it.expect(TK_TO)?;
                let x = it.eval_int(self)?;
                it.expect(TK_COMMA)?;
                let y = it.eval_int(self)?;
                let item = self
                    .menus
                    .item_mut(&path)
                    .ok_or(Exc::Error(ITEM_NOT_DEFINED))?;
                item.x = x as i16 as i32;
                item.y = y as i16 as i32;
                item.flags |= flags::FIXED;
                self.menus.generation += 1;
            }
            MENU_KEY => {
                let path = self.menu_path(it)?;
                let key = if it.peek() == TK_TO {
                    it.pc += 2;
                    match it.eval(self)? {
                        Value::Str(s) => s.first().map(|&c| MenuKey::Ascii(c)),
                        v => {
                            let scan = it.to_int(v)?;
                            let shift = if it.peek() == TK_COMMA {
                                it.pc += 2;
                                it.eval_int(self)?
                            } else {
                                0
                            };
                            if !(0..128).contains(&scan) || !(0..256).contains(&shift) {
                                return err(errors::ILLEGAL_FUNCTION_CALL);
                            }
                            Some(MenuKey::Scancode {
                                scan: scan as u8,
                                shift: shift as u8,
                            })
                        }
                    }
                } else {
                    None
                };
                let item = self
                    .menus
                    .item_mut(&path)
                    .ok_or(Exc::Error(ITEM_NOT_DEFINED))?;
                if !item.children.is_empty() {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                item.key = key;
            }
            ON_MENU => {
                let kind = match it.next_token() {
                    GOTO => OnMenuKind::Goto,
                    GOSUB => OnMenuKind::Gosub,
                    PROC => OnMenuKind::Proc,
                    _ => return err(errors::SYNTAX_ERROR),
                };
                let mut targets = Vec::new();
                loop {
                    targets.push(if kind == OnMenuKind::Proc {
                        it.proc_operand()?
                    } else {
                        it.label_target(self)?
                    });
                    if it.peek() == TK_COMMA {
                        it.pc += 2;
                    } else {
                        break;
                    }
                }
                self.menus.on_menu = Some(OnMenu { kind, targets });
                self.menus.on_menu_armed = false;
            }
            // The original's On Menu Off also arms the jump (a bug kept).
            ON_MENU_ON | ON_MENU_OFF => self.menus.on_menu_armed = self.menus.on_menu.is_some(),
            ON_MENU_DEL => {
                self.menus.on_menu = None;
                self.menus.on_menu_armed = false;
            }
            MENU_ON => self.menus.active = !self.menus.root.children.is_empty(),
            MENU_OFF => self.menus.active = false,
            MENU_MOUSE_ON => self.menus.at_mouse = true,
            MENU_MOUSE_OFF => self.menus.at_mouse = false,
            MENU_BASE => {
                // Sets the position of the first title (+Lib.s:15594).
                let a = it.inst_args(self, kw)?;
                let first = self
                    .menus
                    .root
                    .children
                    .first_mut()
                    .ok_or(Exc::Error(MENU_NOT_OPENED))?;
                if let Some(y) = a.opt(1) {
                    first.y = y as i16 as i32;
                    first.flags |= flags::FIXED;
                }
                if let Some(x) = a.opt(0) {
                    first.x = x as i16 as i32;
                    first.flags |= flags::FIXED;
                }
            }
            MENU_CALC => {
                if self.menus.root.children.is_empty() {
                    return err(MENU_NOT_OPENED);
                }
                self.menu_calc()?;
            }
            MENU_BAR | MENU_LINE | MENU_TLINE | MENU_MOVABLE | MENU_STATIC | MENU_ITEM_MOVABLE
            | MENU_ITEM_STATIC | MENU_ACTIVE | MENU_INACTIVE | MENU_SEPARATE | MENU_LINK
            | MENU_CALLED | MENU_ONCE => {
                let (set, clear) = match kw.token {
                    MENU_BAR => (flags::BAR, flags::TLINE),
                    MENU_LINE => (0, flags::BAR | flags::TLINE),
                    MENU_TLINE => (flags::TLINE, flags::BAR),
                    MENU_MOVABLE => (flags::MOVABLE, 0),
                    MENU_STATIC => (0, flags::MOVABLE),
                    MENU_ITEM_MOVABLE => (flags::ITEM_MOVABLE, 0),
                    MENU_ITEM_STATIC => (0, flags::ITEM_MOVABLE),
                    MENU_ACTIVE => (0, flags::INACTIVE),
                    MENU_INACTIVE => (flags::INACTIVE, 0),
                    MENU_SEPARATE => (flags::SEPARATE, 0),
                    MENU_LINK => (0, flags::SEPARATE),
                    MENU_CALLED => (flags::CALLED, 0),
                    _ => (0, flags::CALLED),
                };
                if it.peek() == TK_PAR1 {
                    let path = self.menu_path(it)?;
                    let item = self
                        .menus
                        .item_mut(&path)
                        .ok_or(Exc::Error(ITEM_NOT_DEFINED))?;
                    item.flags = (item.flags & !clear) | set;
                } else {
                    let level = it.eval_int(self)?;
                    if !(1..=menus::MAX_LEVELS as i32).contains(&level) {
                        return err(errors::ILLEGAL_FUNCTION_CALL);
                    }
                    let f = &mut self.menus.default_flags[level as usize - 1];
                    *f = (*f & !clear) | set;
                }
                self.menus.generation += 1;
            }
            MENU_TO_BANK => {
                let n = it.inst_args(self, kw)?.int(0);
                let data = self
                    .menus
                    .to_bank()
                    .ok_or(Exc::Error(errors::ILLEGAL_FUNCTION_CALL))?;
                if !(1..65536).contains(&n) {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                self.banks.reserve(n as u16, 0, "Menu", true, false);
                if let Some(d) = self.banks.raw_mut(n as u16) {
                    *d = data;
                }
                self.on_banks_changed();
            }
            BANK_TO_MENU => {
                let n = it.inst_args(self, kw)?.int(0);
                self.menus.delete(&[]);
                let cur = self
                    .screens
                    .current
                    .ok_or(Exc::Error(errors::SCREEN_NOT_OPENED))?;
                let bank = (1..65536)
                    .contains(&n)
                    .then(|| self.banks.get(n as u16))
                    .flatten()
                    .ok_or(Exc::Error(errors::BANK_NOT_RESERVED))?;
                if bank.name != "Menu    " {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                let data = bank.raw().unwrap_or(&[]).to_vec();
                if !self.menus.from_bank(&data) {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                self.menus.screen = Some(cur);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// `Choice` (`None`: was a menu item chosen since the last call) /
    /// `Choice(level)`: the item chosen at that level.
    pub(crate) fn choice_fn(&mut self, level: Option<i32>) -> R<i32> {
        let Some(n) = level else {
            let chosen = std::mem::take(&mut self.menus.choice_pending);
            return Ok(if chosen { -1 } else { 0 });
        };
        if !(1..=menus::MAX_LEVELS as i32).contains(&n) {
            return err(errors::ILLEGAL_FUNCTION_CALL);
        }
        Ok(self.menus.choice[n as usize - 1] as i32)
    }

    pub(crate) fn menus_function(&mut self, it: &mut Interp, kw: Keyword) -> R<Option<Value>> {
        if kw.slot != 0 {
            return Ok(None);
        }
        use tk::*;
        let v = match kw.token {
            CHOICE => Value::Int(self.choice_fn(None)?),
            CHOICE_2 => {
                let n = it.func_args(self, kw)?.int(0);
                Value::Int(self.choice_fn(Some(n))?)
            }
            X_MENU | Y_MENU => {
                let path = self.menu_path(it)?;
                let item = self.menus.item(&path).ok_or(Exc::Error(ITEM_NOT_DEFINED))?;
                Value::Int(if kw.token == X_MENU { item.x } else { item.y } as u16 as i32)
            }
            _ => return Ok(None),
        };
        Ok(Some(v))
    }

    /// Reads `(n1,n2,...)`, the path of a menu item.
    fn menu_path(&mut self, it: &mut Interp) -> R<Vec<u16>> {
        it.expect(TK_PAR1)?;
        let mut path = Vec::new();
        loop {
            let n = it.eval_int(self)?;
            if !(1..1024).contains(&n) || path.len() >= menus::MAX_LEVELS {
                return err(errors::ILLEGAL_FUNCTION_CALL);
            }
            path.push(n as u16);
            match it.next_token() {
                TK_COMMA => continue,
                TK_PAR2 => return Ok(path),
                _ => return err(errors::SYNTAX_ERROR),
            }
        }
    }

    /// Menu shortcuts, the menu bar and the On Menu jump, at interpreter
    /// test points (`+ILib.s:940-955`).
    pub(crate) fn menus_test_point(&mut self, it: &mut Interp) -> R<()> {
        // A menu left open by a program that was stopped from outside.
        if self.menu_session.is_some() && self.menus.root.children.is_empty() {
            self.menu_session = None;
        }
        if self.menu_session.is_some() {
            if !self.menu_step()? {
                return Self::menu_freeze(it);
            }
        } else if self.menus.active && !self.menus.root.children.is_empty() {
            if self.input.key_serial != self.menu_key_serial {
                self.menu_key_serial = self.input.key_serial;
                let k = self.input.last_key;
                if let Some(path) = self.menus.find_key(k.ascii, k.raw, k.shift) {
                    self.menus.choose(&path);
                }
            }
            if self.input.mouse_buttons & 2 != 0 {
                self.menu_open()?;
                return Self::menu_freeze(it);
            }
        }
        if !(self.menus.choice_pending && self.menus.on_menu_armed) {
            return Ok(());
        }
        let Some(on) = self.menus.on_menu.clone() else {
            return Ok(());
        };
        let n = self.menus.choice[0] as usize;
        if n == 0 || n > on.targets.len() {
            return Ok(());
        }
        // The jump disarms itself: On Menu On must be used again.
        self.menus.on_menu_armed = false;
        self.menus.choice_pending = false;
        let target = on.targets[n - 1];
        let ret = it.inst_pos;
        match on.kind {
            OnMenuKind::Goto => {
                it.pc = target;
                it.after_jump();
            }
            OnMenuKind::Gosub => {
                it.push_ctl(Ctl::Gosub { ret })?;
                it.pc = target;
            }
            OnMenuKind::Proc => it.call_proc(target, ret, Vec::new())?,
        }
        Err(Exc::Jump)
    }
}

/// Images of the sprite bank (1) or icon bank (2).
fn image_bank_of(banks: &crate::banks::BankSet, icons: bool) -> Option<&Vec<Image>> {
    let b = banks.banks.get(&if icons { 2 } else { 1 })?;
    match &b.data {
        crate::banks::BankData::Images { images, .. } if b.is_icons() == icons => Some(images),
        _ => None,
    }
}

/// The list of items at `parent` (the titles for an empty path).
fn list_of<'a>(menus: &'a menus::Menus, parent: &[u16]) -> &'a [MenuItem] {
    menus.item(parent).map_or(&[][..], |p| &p.children[..])
}

fn child_path(parent: &[u16], n: u16) -> Vec<u16> {
    let mut p = parent.to_vec();
    p.push(n);
    p
}

impl Hardware {
    /// Images for the `(BO n)` / `(IC n)` commands.
    fn menu_images(&self) -> ImageBanks<'_> {
        ImageBanks {
            bobs: self.image_bank(false),
            icons: self.image_bank(true),
        }
    }

    /// The menu screen (`MnAdEc`), "Screen not opened" if it was closed.
    fn menu_screen(&self) -> R<usize> {
        match self.menus.screen {
            Some(n) if self.screens.get(n).is_some() => Ok(n),
            _ => err(errors::SCREEN_NOT_OPENED),
        }
    }

    /// The drawing state while the menu is shown (`StaMn`, +W.s:4987):
    /// normal writing, outline on, full lines, the whole screen as clip.
    fn menu_gr(&self, n: usize) -> GrState {
        let s = self.screens.get(n).unwrap();
        let mut gr = s.gr.clone();
        gr.writing = JAM2;
        gr.paint_outline = true;
        gr.line_pattern = 0xFFFF;
        gr.line_count = 15;
        gr.clip = (0, 0, s.width as i32, s.height as i32);
        gr
    }

    /// `Menu Calc` / `MnCalc`: positions of the whole tree.
    fn menu_calc(&mut self) -> R<()> {
        let n = self.menu_screen()?;
        let mut gr = self.menu_gr(n);
        let mut root = std::mem::take(&mut self.menus.root);
        calc_list(&mut root.children, 0, 0, &mut gr, &self.menu_images());
        self.menus.root = root;
        self.menus.calc_generation = self.menus.generation;
        Ok(())
    }

    /// Opens the menu (`MnGere` up to `MnLoop`, +Lib.s:15782).
    fn menu_open(&mut self) -> R<()> {
        let n = self.menu_screen()?;
        if self.menus.calc_generation != self.menus.generation {
            self.menu_calc()?;
        }
        let amal_frozen = self.sprites.amal.frozen_all;
        self.sprites.amal.frozen_all = true;
        let base = if self.menus.at_mouse {
            let s = self.screens.get(n).unwrap();
            (
                s.x_screen(self.input.mouse_x),
                s.y_screen(self.input.mouse_y),
            )
        } else {
            (0, 0)
        };
        self.menu_session = Some(Box::new(MenuSession {
            screen: n,
            table: Vec::new(),
            act: None,
            zones: Vec::new(),
            saves: HashMap::new(),
            base,
            gr: self.menu_gr(n),
            drag: None,
            amal_frozen,
        }));
        self.menu_branch(&[], 0, base);
        Ok(())
    }

    /// Stops the program while the menu is open: loops and jumps wait at
    /// their test point; an instruction that was already waiting (Wait,
    /// Wait Vbl...) is retried at the next frame, its delay pushed back.
    /// The program is stopped while the menu is open (the original runs
    /// the whole menu interaction inside `Test_Force`): the instruction at
    /// the test point is executed again later; a wait in progress is
    /// extended so it does not expire while the menu is open.
    fn menu_freeze(it: &mut Interp) -> R<()> {
        let pos = it.inst_pos;
        if let Some(w) = it.wait.as_mut()
            && w.pos == pos
            && let WaitKind::Vbl(until) = &mut w.kind
        {
            *until += 1;
        }
        Err(Exc::Block)
    }

    /// Runs `f` with the menu drawing state and a canvas on the displayed
    /// bitmap of the menu screen.
    fn menu_canvas<T>(
        &mut self,
        f: impl FnOnce(&mut GrState, &mut Canvas, &ImageBanks) -> T,
    ) -> Option<T> {
        let sess = self.menu_session.as_mut()?;
        let images = ImageBanks {
            bobs: image_bank_of(&self.banks, false),
            icons: image_bank_of(&self.banks, true),
        };
        let s = self.screens.get_mut(sess.screen)?;
        let (w, h, planes, bm) = (s.width, s.height, s.planes, s.physic);
        s.version += 1;
        let mut c = Canvas::new(&mut s.bitmaps[bm], w, h, planes);
        Some(f(&mut sess.gr, &mut c, &images))
    }

    /// Draws the item at `path` at its position (`MnDraw` in draw mode).
    fn menu_draw(&mut self, path: &[u16], active: bool, background: bool) {
        let Some(item) = self.menus.item(path).cloned() else {
            return;
        };
        let mode = DrawMode {
            calc: false,
            active,
            background,
        };
        self.menu_canvas(|gr, c, img| draw_item(&item, (item.xx, item.yy), mode, gr, Some(c), img));
    }

    /// Saves the background of a rectangle (inclusive) for the branch
    /// starting at `head` (`MnSave`).
    fn menu_save(&mut self, head: Vec<u16>, r: (i32, i32, i32, i32)) {
        let Some(sess) = self.menu_session.as_mut() else {
            return;
        };
        let Some(s) = self.screens.get(sess.screen) else {
            return;
        };
        let (sw, sh) = (s.width as i32, s.height as i32);
        let (x1, y1) = (r.0.max(0), r.1.max(0));
        let (x2, y2) = ((r.2 + 1).min(sw), (r.3 + 1).min(sh));
        if x1 >= sw || y1 >= sh || x2 <= x1 || y2 <= y1 {
            sess.saves.remove(&head);
            return;
        }
        let bm = &s.bitmaps[s.physic];
        let mut pixels = Vec::with_capacity(((x2 - x1) * (y2 - y1)) as usize);
        for y in y1..y2 {
            pixels.extend_from_slice(&bm[(y * sw + x1) as usize..(y * sw + x2) as usize]);
        }
        let saved = Saved {
            x: x1,
            y: y1,
            w: (x2 - x1) as usize,
            h: (y2 - y1) as usize,
            pixels,
        };
        sess.saves.insert(head, saved);
    }

    /// Puts back a saved background (`MnRest`).
    fn menu_restore(&mut self, head: &[u16]) {
        let Some(sess) = self.menu_session.as_mut() else {
            return;
        };
        let Some(sv) = sess.saves.remove(head) else {
            return;
        };
        let Some(s) = self.screens.get_mut(sess.screen) else {
            return;
        };
        let sw = s.width as usize;
        let bm = s.physic;
        for j in 0..sv.h {
            let o = (sv.y as usize + j) * sw + sv.x as usize;
            s.bitmaps[bm][o..o + sv.w].copy_from_slice(&sv.pixels[j * sv.w..(j + 1) * sv.w]);
        }
        s.version += 1;
    }

    /// Draws the items of list `parent` from `start` and sets their zones
    /// (`MnBranch`, +Lib.s:16420).
    fn menu_branch(&mut self, parent: &[u16], start: usize, base: (i32, i32)) {
        let Some(sn) = self.menu_session.as_ref().map(|s| s.screen) else {
            return;
        };
        let sw = self.screens.get(sn).map_or(320, |s| s.width as i32);
        let level = parent.len();
        let (mut i, mut base) = (start, base);
        loop {
            let Some(p) = self.menus.item_mut(parent) else {
                return;
            };
            if i >= p.children.len() {
                return;
            }
            let (r, end) = maxi(&mut p.children, i, base, sw);
            let head = p.children[i].clone();
            let items: Vec<(Vec<u16>, ZoneRect)> = p.children[i..end]
                .iter()
                .map(|c| {
                    (
                        child_path(parent, c.number),
                        (c.xx.max(0), c.yy.max(0), c.xx + c.tx, c.yy + c.ty),
                    )
                })
                .collect();
            let prev_pos = (p.children[end - 1].xx, p.children[end - 1].yy);
            let more = end < p.children.len();
            self.menu_save(child_path(parent, head.number), r);
            if head.objects[3].is_none() {
                // The frame of the branch: filled with ink B, outlined with
                // ink C.
                let [_, b1, c1, ..] = head.inks;
                self.menu_canvas(|gr, c, _| {
                    gr.pattern = None;
                    gr.pattern_number = 0;
                    gr.line_pattern = 0xFFFF;
                    gr.ink = b1;
                    gr.outline = c1;
                    gr.writing = JAM2;
                    gr.bar(c, r.0, r.1, r.2 - 1, r.3 - 1);
                });
            }
            for (k, (path, rect)) in items.into_iter().enumerate() {
                self.menu_draw(&path, false, true);
                if let Some(sess) = self.menu_session.as_mut() {
                    sess.zones.retain(|z| z.path != path);
                    sess.zones.push(Zone {
                        level,
                        index: i + k,
                        path,
                        rect,
                    });
                }
            }
            if !more {
                return;
            }
            base = prev_pos;
            i = end;
        }
    }

    /// Removes the zones of list `parent` from `start` and restores the
    /// backgrounds, last branch first (`MnEBranch`).
    fn menu_ebranch(&mut self, parent: &[u16], start: usize) {
        let numbers: Vec<u16> = list_of(&self.menus, parent)
            .iter()
            .skip(start)
            .map(|c| c.number)
            .collect();
        if let Some(sess) = self.menu_session.as_mut() {
            sess.zones.retain(|z| {
                !(z.level == parent.len() && z.path.starts_with(parent) && z.index >= start)
            });
        }
        for n in numbers.into_iter().rev() {
            self.menu_restore(&child_path(parent, n));
        }
    }

    /// Closes the sub-menus above `level` (`MnDEff`): the parent of the
    /// last closed one becomes the highlighted item.
    fn menu_deff(&mut self, level: usize) {
        loop {
            let Some(sess) = self.menu_session.as_mut() else {
                return;
            };
            if sess.table.len() <= level {
                return;
            }
            let p = sess.table.pop().unwrap();
            sess.act = Some(p.clone());
            self.menu_ebranch(&p, 0);
        }
    }

    /// Item under the mouse (`GetZone` on the menu zones): deeper levels
    /// first, then the last items. Returns its path and level.
    fn menu_zone_at_mouse(&self) -> Option<(Vec<u16>, usize)> {
        let sess = self.menu_session.as_ref()?;
        let (hx, hy) = (self.input.mouse_x, self.input.mouse_y);
        if self.screens.screen_at(hx, hy, None, 8) != Some(sess.screen) {
            return None;
        }
        let s = self.screens.get(sess.screen)?;
        let (x, y) = (s.x_screen(hx), s.y_screen(hy));
        sess.zones
            .iter()
            .filter(|z| x >= z.rect.0 && y >= z.rect.1 && x <= z.rect.2 && y <= z.rect.3)
            .max_by_key(|z| (z.level, z.index))
            .map(|z| (z.path.clone(), z.level))
    }

    /// One iteration of the menu loop (`MnLoop`, +Lib.s:15838). Returns
    /// true when the menu has been closed.
    fn menu_step(&mut self) -> R<bool> {
        if self
            .screens
            .get(self.menu_session.as_ref().unwrap().screen)
            .is_none()
        {
            self.menu_session = None;
            return err(errors::SCREEN_NOT_OPENED);
        }
        if self.menu_session.as_ref().unwrap().drag.is_some() {
            self.menu_drag_step();
            return Ok(false);
        }
        // Items with Menu Called are drawn again at each VBL.
        let table = self.menu_session.as_ref().unwrap().table.clone();
        let mut parents: Vec<Vec<u16>> = vec![Vec::new()];
        parents.extend(table.iter().cloned());
        for (level, parent) in parents.iter().enumerate() {
            for item in list_of(&self.menus, parent).to_vec() {
                if item.flags & flags::CALLED != 0 {
                    let path = child_path(parent, item.number);
                    let active = table.get(level) == Some(&path);
                    self.menu_draw(&path, active, false);
                }
            }
        }
        let depth = table.len();
        // Shift pressed: the menu ignores the mouse.
        let mut hit = if self.input.shifts() & 3 != 0 {
            None
        } else {
            self.menu_zone_at_mouse()
        };
        let buttons = self.input.mouse_buttons;
        if buttons & 2 == 0 {
            self.menu_exit();
            return Ok(true);
        }
        if buttons & 1 != 0 {
            if let Some((path, level)) = hit {
                self.menu_drag_start(path, level);
            }
            return Ok(false);
        }
        if let Some((path, _)) = &hit {
            let inactive = self
                .menus
                .item(path)
                .is_some_and(|i| i.flags & flags::INACTIVE != 0);
            // An item whose sub-menu is open is not highlighted again.
            if inactive || table.contains(path) {
                hit = None;
            }
        }
        let act = self.menu_session.as_ref().unwrap().act.clone();
        if hit.as_ref().map(|h| &h.0) == act.as_ref() {
            return Ok(false);
        }
        let level = hit.as_ref().map_or(depth, |h| h.1);
        self.menu_deff(level);
        if let Some(old) = self.menu_session.as_mut().unwrap().act.take() {
            self.menu_draw(&old, false, true);
        }
        let Some((path, _)) = hit else {
            return Ok(false);
        };
        self.menu_session.as_mut().unwrap().act = Some(path.clone());
        self.menu_draw(&path, true, true);
        let Some(item) = self.menus.item(&path) else {
            return Ok(false);
        };
        if !item.children.is_empty() {
            let base = (item.xx, item.yy);
            let sess = self.menu_session.as_mut().unwrap();
            sess.table.push(path.clone());
            self.menu_branch(&path, 0, base);
            self.menu_session.as_mut().unwrap().act = None;
        }
        Ok(false)
    }

    /// Button released: records the choice and closes the menu
    /// (`MnExit` / `MnEnd`).
    fn menu_exit(&mut self) {
        let sess = self.menu_session.as_ref().unwrap();
        let choice = sess.act.as_ref().map(|act| {
            let mut c: Vec<u16> = sess.table.iter().map(|p| *p.last().unwrap()).collect();
            c.push(*act.last().unwrap());
            c
        });
        self.menus.choice = [0; menus::MAX_LEVELS];
        self.menus.choice_pending = false;
        if let Some(c) = choice {
            self.menus.choose(&c);
        }
        self.menu_close();
    }

    /// Erases the whole menu (`MnEnd`).
    fn menu_close(&mut self) {
        if self.menu_session.is_none() {
            return;
        }
        self.menu_deff(0);
        self.menu_ebranch(&[], 0);
        let sess = self.menu_session.take().unwrap();
        self.sprites.amal.frozen_all = sess.amal_frozen;
    }

    /// Left button on an item: starts moving it (`MnBGoch`, +Lib.s:15946).
    fn menu_drag_start(&mut self, path: Vec<u16>, level: usize) {
        self.menu_deff(level);
        if let Some(old) = self.menu_session.as_mut().unwrap().act.take() {
            self.menu_draw(&old, false, true);
        }
        let (parent, n) = path.split_at(path.len() - 1);
        let list = list_of(&self.menus, parent);
        let Some(idx) = list.iter().position(|c| c.number == n[0]) else {
            return;
        };
        let item = &list[idx];
        let s = self
            .screens
            .get(self.menu_session.as_ref().unwrap().screen)
            .unwrap();
        let (sw, sh) = (s.width as i32, s.height as i32);
        let (mx, my) = (
            s.x_screen(self.input.mouse_x),
            s.y_screen(self.input.mouse_y),
        );
        let (head, lim, size) = if idx == 0 || item.flags & flags::SEPARATE != 0 {
            // The whole branch moves, anywhere in the screen.
            if item.flags & flags::MOVABLE == 0 {
                return;
            }
            let (a3, a4) = (item.mx, item.my);
            (
                path.clone(),
                (-a3 + 1, -a4 + 1, sw + a3 - 2, sh + a4 - 2),
                (a3, a4),
            )
        } else {
            // One item moves inside its branch.
            if item.flags & flags::ITEM_MOVABLE == 0 {
                return;
            }
            let h = (0..idx)
                .rev()
                .find(|&k| k == 0 || list[k].flags & flags::SEPARATE != 0)
                .unwrap();
            let hd = &list[h];
            (
                child_path(parent, hd.number),
                (hd.xx, hd.yy, hd.xx + hd.mx, hd.yy + hd.my),
                (item.tx, item.ty),
            )
        };
        let off = (item.xx - mx, item.yy - my);
        let pos = (item.xx, item.yy);
        self.menu_session.as_mut().unwrap().drag = Some(Drag {
            item: path,
            head,
            off,
            lim,
            size,
            shown: None,
            line: 0x00FF,
            pos,
        });
        self.menu_drag_step();
    }

    /// Draws or erases the outline of a moved item (complement mode).
    fn menu_drag_outline(&mut self, r: (i32, i32, i32, i32), gr: &GrState) {
        let mut g = gr.clone();
        self.menu_canvas(|_, c, _| {
            g.x = r.0;
            g.y = r.1;
            let pts = [(r.2, r.1), (r.2, r.3), (r.0, r.3), (r.0, r.1)];
            g.poly_draw(c, &pts, false, 31);
        });
    }

    /// One VBL of the outline loop (`MnMgL`), then the move (`MnMgR`).
    fn menu_drag_step(&mut self) {
        let mut d = self.menu_session.as_mut().unwrap().drag.take().unwrap();
        if let Some((r, g)) = d.shown.take() {
            self.menu_drag_outline(r, &g);
        }
        if self.input.mouse_buttons & 3 == 3 {
            let s = self
                .screens
                .get(self.menu_session.as_ref().unwrap().screen)
                .unwrap();
            let (mx, my) = (
                s.x_screen(self.input.mouse_x),
                s.y_screen(self.input.mouse_y),
            );
            let (mut x0, mut y0) = ((mx + d.off.0).max(d.lim.0), (my + d.off.1).max(d.lim.1));
            if x0 + d.size.0 > d.lim.2 {
                x0 = d.lim.2 - d.size.0;
            }
            if y0 + d.size.1 > d.lim.3 {
                y0 = d.lim.3 - d.size.1;
            }
            d.pos = (x0, y0);
            let mut g = self.menu_session.as_ref().unwrap().gr.clone();
            g.ink = 31;
            g.paper = 0;
            g.writing = JAM2 | COMPLEMENT;
            g.line_pattern = d.line;
            g.line_count = 15;
            let r = (x0, y0, x0 + d.size.0 - 1, y0 + d.size.1 - 1);
            self.menu_drag_outline(r, &g);
            d.shown = Some((r, g));
            d.line = d.line.rotate_left(1);
            self.menu_session.as_mut().unwrap().drag = Some(d);
            return;
        }
        // Released: the item moves, the next one keeps its place.
        let (parent, n) = d.item.split_at(d.item.len() - 1);
        let parent = parent.to_vec();
        let Some(list) = self.menus.item_mut(&parent).map(|p| &mut p.children) else {
            return;
        };
        let Some(idx) = list.iter().position(|c| c.number == n[0]) else {
            return;
        };
        let (dx, dy) = (d.pos.0 - list[idx].xx, d.pos.1 - list[idx].yy);
        list[idx].x += dx;
        list[idx].y += dy;
        list[idx].flags |= flags::FIXED;
        let group_head = idx == 0 || list[idx].flags & flags::SEPARATE != 0;
        let next = if group_head {
            (idx + 1..list.len()).find(|&k| list[k].flags & flags::SEPARATE != 0)
        } else {
            (idx + 1 < list.len()).then_some(idx + 1)
        };
        if let Some(k) = next {
            list[k].x -= dx;
            list[k].y -= dy;
            list[k].flags |= flags::FIXED;
        }
        let head_n = *d.head.last().unwrap();
        let hidx = list.iter().position(|c| c.number == head_n).unwrap_or(0);
        let base = if hidx > 0 {
            (list[hidx - 1].xx, list[hidx - 1].yy)
        } else if let Some(p) = self.menus.item(&parent).filter(|_| !parent.is_empty()) {
            (p.xx, p.yy)
        } else {
            self.menu_session.as_ref().unwrap().base
        };
        self.menu_ebranch(&parent, hidx);
        self.menu_branch(&parent, hidx, base);
    }
}

#[cfg(test)]
mod tests {
    use crate::Machine;
    use crate::input::InputEvent;
    use crate::interp::RunState;
    use crate::menus::{MenuKey, flags};

    #[test]
    fn menu_shortcut_triggers_on_menu_gosub() {
        let src = "Menu$(1)=\"Project\"\nMenu$(1,1)=\"Quit\"\nMenu Key(1,1) To \"q\"\nMenu On\n\
                   On Menu Gosub HANDLER\nOn Menu On\nDo\nWait Vbl\nLoop\n\
                   HANDLER: Print Choice(1);Choice(2) : End";
        let prg = crate::tokenise::tokenise_program(src.as_bytes()).unwrap();
        let mut m = Machine::new();
        m.run_program(&prg).unwrap();
        for _ in 0..5 {
            m.vbl();
        }
        m.input(InputEvent::Key {
            scancode: 0x10,
            pressed: true,
            ch: Some('q'),
        });
        for _ in 0..5 {
            m.vbl();
        }
        assert!(matches!(m.state, RunState::Stopped(_)), "{:?}", m.state);
        assert!(m.interp.prg.is_some());
    }

    fn machine(src: &str) -> Machine {
        let prg = crate::tokenise::tokenise_program(src.as_bytes()).unwrap();
        let mut m = Machine::new();
        m.run_program(&prg).unwrap();
        m
    }

    /// Moves the mouse to a hardware position with the given buttons and
    /// runs `frames` VBLs.
    fn mouse(m: &mut Machine, x: i32, y: i32, buttons: u8, frames: usize) {
        use crate::display::{HW_X0, HW_Y0};
        use crate::input::MouseButton;
        m.input(InputEvent::MouseMove {
            x: ((x - HW_X0) * 2) as f32,
            y: ((y - HW_Y0) * 2) as f32,
        });
        let old = m.hw.input.mouse_buttons;
        for (bit, button) in [(1, MouseButton::Left), (2, MouseButton::Right)] {
            if (old ^ buttons) & bit != 0 {
                m.input(InputEvent::MouseButton {
                    button,
                    pressed: buttons & bit != 0,
                });
            }
        }
        for _ in 0..frames {
            m.vbl();
        }
    }

    /// Default screen at (128,42): screen x,y to hardware.
    fn hw(x: i32, y: i32) -> (i32, i32) {
        (128 + x, 42 + y)
    }

    const MENU: &str = "Curs Off : Cls 0\nMenu$(1)=\" File \" : Menu$(2)=\" Edit \"\n\
                        Menu$(1,1)=\" Load \" : Menu$(1,2)=\" Save \" : Menu$(1,2,1)=\" As \"\n\
                        Menu$(2,1)=\" Cut \" : Menu Inactive(2,1)\nMenu On\n";

    #[test]
    fn select_with_the_mouse() {
        let mut m = machine(&format!("{MENU}C=0\nDo\nInc C\nIf Choice Then End\nLoop"));
        mouse(&mut m, 300, 200, 0, 3);
        let before = m.hw.screens.get(0).unwrap().bitmaps[0].clone();
        // Open: the bar is drawn in ink B (pen 2) at the top of the screen.
        mouse(&mut m, 300, 200, 2, 2);
        assert!(m.hw.menu_session.is_some());
        assert_eq!(m.hw.screens.get(0).unwrap().pixel(100, 1), Some(2));
        let c = m.interp.globals.clone();
        // The program does not run while the menu is open.
        mouse(&mut m, 300, 200, 2, 3);
        assert_eq!(format!("{:?}", m.interp.globals), format!("{c:?}"));
        // Title 1 at (2,2) 48x8, its menu below from (4,12); item 2 has a
        // sub-menu on its right.
        let (x, y) = hw(10, 5);
        mouse(&mut m, x, y, 2, 2);
        let (x, y) = hw(10, 23);
        mouse(&mut m, x, y, 2, 2);
        let s = m.hw.menu_session.as_ref().unwrap();
        assert_eq!(s.table, vec![vec![1], vec![1, 2]]);
        let item = m.hw.menus.item(&[1, 2, 1]).unwrap();
        let (x, y) = (128 + item.xx + 4, 42 + item.yy + 4);
        mouse(&mut m, x, y, 2, 2);
        assert_eq!(m.hw.menu_session.as_ref().unwrap().act, Some(vec![1, 2, 1]));
        mouse(&mut m, x, y, 0, 3);
        assert!(m.hw.menu_session.is_none());
        assert_eq!(m.hw.screens.get(0).unwrap().bitmaps[0], before);
        assert!(matches!(m.state, RunState::Stopped(_)), "{:?}", m.state);
        assert_eq!(m.hw.menus.choice[..3], [1, 2, 1]);
    }

    #[test]
    fn inactive_items_and_release_outside() {
        let mut m = machine(&format!("{MENU}Do : Wait Vbl : Loop"));
        mouse(&mut m, 300, 200, 2, 2);
        let t2 = m.hw.menus.item(&[2]).unwrap().xx;
        let (x, y) = hw(t2 + 4, 5);
        mouse(&mut m, x, y, 2, 2);
        let (x, y) = hw(t2 + 8, 15);
        mouse(&mut m, x, y, 2, 2);
        assert_eq!(m.hw.menu_session.as_ref().unwrap().act, None);
        mouse(&mut m, x, y, 0, 2);
        assert!(m.hw.menu_session.is_none());
        assert!(!m.hw.menus.choice_pending);
        assert_eq!(m.hw.menus.choice[0], 0);
    }

    #[test]
    fn drag_the_menu_bar() {
        let mut m = machine(&format!("{MENU}Do : Wait Vbl : Loop"));
        mouse(&mut m, 300, 200, 2, 2);
        let (x, y) = hw(10, 5);
        mouse(&mut m, x, y, 3, 2);
        assert!(m.hw.menu_session.as_ref().unwrap().drag.is_some());
        mouse(&mut m, x + 20, y + 30, 3, 2);
        mouse(&mut m, x + 20, y + 30, 2, 2);
        let t = m.hw.menus.item(&[1]).unwrap();
        assert_eq!((t.x, t.y, t.xx, t.yy), (22, 32, 22, 32));
        assert!(t.flags & flags::FIXED != 0);
        mouse(&mut m, x + 20, y + 30, 0, 2);
        assert!(m.hw.menu_session.is_none());
    }

    #[test]
    fn menu_banks() {
        let src = format!(
            "{MENU}Menu Key(1,1) To \"l\"\nMenu To Bank 5\nMenu Del\nBank To Menu 5\nPrint Menu$(1,2,1);\n"
        );
        // Menu$ cannot be read; check the tree instead.
        let mut m = machine(&src.replace("Print Menu$(1,2,1);\n", ""));
        for _ in 0..3 {
            m.vbl();
        }
        assert!(matches!(m.state, RunState::Stopped(_)), "{:?}", m.state);
        assert_eq!(m.hw.banks.get(5).unwrap().name, "Menu    ");
        assert!(m.hw.menus.item(&[1, 2, 1]).is_some());
        assert_eq!(
            m.hw.menus.item(&[1, 1]).unwrap().key,
            Some(MenuKey::Ascii(b'l'))
        );
        assert!(!m.hw.menus.active);
        let m = machine("Reserve As Work 3,100\nBank To Menu 3");
        let mut m = m;
        m.vbl();
        assert!(
            matches!(&m.state, RunState::Stopped(i) if i.reason == crate::interp::StopReasonOrError::Error(23))
        );
    }
}
