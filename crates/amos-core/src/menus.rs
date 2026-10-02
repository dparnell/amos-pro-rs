//! AMOS menus: the menu tree built by `Menu$`, the compiled item strings
//! (`MnObjet`, `+Lib.s:17435`) and the choice state.
//!
//! Drawing and mouse interaction live in `machine/inst_menus.rs`.

/// Maximum menu depth (`MnNDim`).
pub const MAX_LEVELS: usize = 8;

/// Flag bits of a menu item (`MnFlag`).
pub mod flags {
    pub const FLAT: u16 = 1 << 0;
    pub const FIXED: u16 = 1 << 1;
    pub const SEPARATE: u16 = 1 << 2;
    pub const BAR: u16 = 1 << 3;
    pub const INACTIVE: u16 = 1 << 4;
    pub const TLINE: u16 = 1 << 5;
    pub const MOVABLE: u16 = 1 << 6;
    pub const ITEM_MOVABLE: u16 = 1 << 7;
    /// `Menu Called`: redraw the item at each VBL while the menu is open.
    pub const CALLED: u16 = 1 << 8;
}

/// One drawing command of a compiled item string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuOp {
    /// Plain text at the cursor.
    Text(Vec<u8>),
    /// `(BA x,y)` filled rectangle from the cursor to x,y.
    Bar(i16, i16),
    /// `(LI x,y)` line from the cursor.
    Line(i16, i16),
    /// `(EL rx,ry)` ellipse at the cursor.
    Ellipse(i16, i16),
    /// `(PA n)` fill pattern.
    Pattern(i16),
    /// `(IN n,c)` ink n (1 = ink, 2 = paper, 3 = outline) = colour c.
    Ink(i16, i16),
    /// `(BO n)` paste bob image.
    Bob(i16),
    /// `(IC n)` paste icon.
    Icon(i16),
    /// `(LO x,y)` move the cursor.
    Locate(i16, i16),
    /// `(OU f)` outline on/off.
    Outline(i16),
    /// `(SL n)` line pattern.
    SetLine(i16),
    /// `(SF n)` font.
    SetFont(i16),
    /// `(PR name)` call a procedure to draw.
    Proc(Vec<u8>),
    /// `(RE n)` reserve bytes of data for the procedure.
    Reserve(i16),
    /// `(SS n)` text style.
    Style(i16),
}

/// Two letter command names, in opcode order (`MnOToken`).
const COMMANDS: [(&[u8; 2], i8); 14] = [
    (b"BA", 2),
    (b"LI", 2),
    (b"EL", 2),
    (b"PA", 1),
    (b"IN", 2),
    (b"BO", 1),
    (b"IC", 1),
    (b"LO", 2),
    (b"OU", 1),
    (b"SL", 1),
    (b"SF", 1),
    (b"PR", -1),
    (b"RE", 1),
    (b"SS", 1),
];

/// Compiles a menu item string such as `"(BA 20,10)Load(LO 2,1)"`.
/// Returns `None` on a syntax error.
pub fn compile(s: &[u8]) -> Option<Vec<MenuOp>> {
    let mut ops = Vec::new();
    let mut i = 0;
    while i < s.len() {
        // Text up to the next '('; control characters are dropped and ESC
        // skips itself and two more bytes.
        let mut text = Vec::new();
        while i < s.len() && s[i] != b'(' {
            match s[i] {
                27 => i += 3,
                c if c < 32 => i += 1,
                c => {
                    text.push(c);
                    i += 1;
                }
            }
        }
        if !text.is_empty() {
            ops.push(MenuOp::Text(text));
        }
        if i >= s.len() {
            break;
        }
        // Commands inside the parentheses, separated by ':'.
        i += 1;
        loop {
            skip_spaces(s, &mut i);
            let c1 = s.get(i)?.to_ascii_uppercase();
            let c2 = s.get(i + 1)?.to_ascii_uppercase();
            i += 2;
            while i < s.len() && s[i].is_ascii_alphabetic() {
                i += 1;
            }
            let (idx, nargs) = COMMANDS
                .iter()
                .enumerate()
                .find(|(_, (n, _))| **n == [c1, c2])
                .map(|(k, (_, a))| (k, *a))?;
            if nargs < 0 {
                skip_spaces(s, &mut i);
                let start = i;
                while i < s.len() && s[i] != b')' && s[i] != b':' {
                    i += 1;
                }
                let name: Vec<u8> = s[start..i].iter().copied().filter(|c| *c != b' ').collect();
                if name.is_empty() {
                    return None;
                }
                ops.push(MenuOp::Proc(name));
            } else {
                let mut args = [0i16; 2];
                for (k, arg) in args.iter_mut().enumerate().take(nargs as usize) {
                    if k > 0 {
                        skip_spaces(s, &mut i);
                        if s.get(i) != Some(&b',') {
                            return None;
                        }
                        i += 1;
                    }
                    *arg = number(s, &mut i)?;
                }
                let [a, b] = args;
                ops.push(match idx {
                    0 => MenuOp::Bar(a, b),
                    1 => MenuOp::Line(a, b),
                    2 => MenuOp::Ellipse(a, b),
                    3 => MenuOp::Pattern(a),
                    4 => MenuOp::Ink(a, b),
                    5 => MenuOp::Bob(a),
                    6 => MenuOp::Icon(a),
                    7 => MenuOp::Locate(a, b),
                    8 => MenuOp::Outline(a),
                    9 => MenuOp::SetLine(a),
                    10 => MenuOp::SetFont(a),
                    12 => MenuOp::Reserve(a),
                    _ => MenuOp::Style(a),
                });
            }
            skip_spaces(s, &mut i);
            match s.get(i) {
                Some(b':') => i += 1,
                Some(b')') => {
                    i += 1;
                    break;
                }
                _ => return None,
            }
        }
    }
    Some(ops)
}

fn skip_spaces(s: &[u8], i: &mut usize) {
    while *i < s.len() && s[*i] <= 32 {
        *i += 1;
    }
}

fn number(s: &[u8], i: &mut usize) -> Option<i16> {
    skip_spaces(s, i);
    let neg = s.get(*i) == Some(&b'-');
    if neg {
        *i += 1;
        skip_spaces(s, i);
    }
    let start = *i;
    let mut v: i32 = 0;
    while let Some(c) = s.get(*i).filter(|c| c.is_ascii_digit()) {
        v = v.wrapping_mul(10).wrapping_add((c - b'0') as i32);
        *i += 1;
    }
    if *i == start {
        return None;
    }
    Some(if neg { -v } else { v } as i16)
}

/// Keyboard shortcut of a menu item (`Menu Key`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKey {
    Ascii(u8),
    Scancode { scan: u8, shift: u8 },
}

/// One menu item and its sub-menu (a `MnLong` node, `+Equ.s:779`).
#[derive(Clone, Debug, Default)]
pub struct MenuItem {
    /// Number of the item at its level (1..1023).
    pub number: u16,
    pub flags: u16,
    /// Position relative to the previous item of its group, or to the
    /// parent for the first one (`MnX`/`MnY`, set by `Set Menu` or
    /// computed by `MnCalc`).
    pub x: i32,
    pub y: i32,
    /// Size of the item (`MnTx`/`MnTy`).
    pub tx: i32,
    pub ty: i32,
    /// Size of the group it starts (`MnMX`/`MnMY`, first item of a group).
    pub mx: i32,
    pub my: i32,
    /// Absolute position in the menu screen (`MnXX`/`MnYY`).
    pub xx: i32,
    pub yy: i32,
    /// Compiled strings: normal, selected, inactive, background.
    pub objects: [Option<Vec<MenuOp>>; 4],
    pub key: Option<MenuKey>,
    /// Inks of the item (`MnInkA1..C2`): A,B,C for the normal object then
    /// for the selected one (taken from the text window paper and pen).
    pub inks: [u8; 6],
    /// Sub items, sorted by number.
    pub children: Vec<MenuItem>,
}

impl MenuItem {
    pub fn child(&self, n: u16) -> Option<&MenuItem> {
        self.children.iter().find(|c| c.number == n)
    }

    /// Finds or creates the child `n`, keeping children sorted.
    pub fn child_mut_or_insert(&mut self, n: u16, flags: u16) -> &mut MenuItem {
        let pos = match self.children.binary_search_by_key(&n, |c| c.number) {
            Ok(p) => p,
            Err(p) => {
                self.children.insert(
                    p,
                    MenuItem {
                        number: n,
                        flags,
                        ..Default::default()
                    },
                );
                p
            }
        };
        &mut self.children[pos]
    }
}

/// All menu state of a program.
#[derive(Clone, Debug)]
pub struct Menus {
    /// The menu bar: the root's children are the titles (`MnBase` is the
    /// first of them).
    pub root: MenuItem,
    /// Default flags of new items for each level (`MnDFlags`).
    pub default_flags: [u16; MAX_LEVELS],
    /// `Menu On`: the right mouse button opens the menu (`BitMenu`).
    pub active: bool,
    /// `Menu Mouse On`: the menu appears at the mouse position.
    pub at_mouse: bool,
    /// Screen the menu is drawn in (`MnAdEc`): the current screen when the
    /// first item was defined.
    pub screen: Option<usize>,
    /// Item numbers of the last choice for each level (`Choice(n)`).
    pub choice: [u16; MAX_LEVELS],
    /// A choice was made and not read by `=Choice` yet.
    pub choice_pending: bool,
    /// `On Menu Goto|Gosub|Proc` targets and whether the jump is armed.
    pub on_menu: Option<OnMenu>,
    pub on_menu_armed: bool,
    /// Incremented on every change (`MnChange`): positions are computed
    /// again when the menu opens.
    pub generation: u64,
    /// Generation of the last position computation.
    pub calc_generation: u64,
}

impl Default for Menus {
    fn default() -> Self {
        // MenuReset (+Lib.s:17279): the bar is a total line, the other
        // levels vertical bars, all movable.
        let mut default_flags = [flags::BAR | flags::MOVABLE; MAX_LEVELS];
        default_flags[0] = flags::TLINE | flags::MOVABLE;
        Menus {
            root: MenuItem::default(),
            default_flags,
            active: false,
            at_mouse: false,
            screen: None,
            choice: [0; MAX_LEVELS],
            choice_pending: false,
            on_menu: None,
            on_menu_armed: false,
            generation: 1,
            calc_generation: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OnMenuKind {
    Goto,
    Gosub,
    Proc,
}

#[derive(Clone, Debug)]
pub struct OnMenu {
    pub kind: OnMenuKind,
    /// Label positions (Goto/Gosub) or procedure numbers (Proc).
    pub targets: Vec<usize>,
}

impl Menus {
    /// Item at `path` (1-based numbers per level).
    pub fn item(&self, path: &[u16]) -> Option<&MenuItem> {
        let mut it = &self.root;
        for &n in path {
            it = it.child(n)?;
        }
        Some(it)
    }

    pub fn item_mut(&mut self, path: &[u16]) -> Option<&mut MenuItem> {
        let mut it = &mut self.root;
        for &n in path {
            it = it.children.iter_mut().find(|c| c.number == n)?;
        }
        Some(it)
    }

    /// Finds or creates the item at `path` (`MnFind` / `MnIns`): the
    /// parent must exist. New items get the default flags of their level.
    pub fn item_create(&mut self, path: &[u16]) -> Option<&mut MenuItem> {
        let (&last, parent) = path.split_last()?;
        let flags = self.default_flags[(path.len() - 1).min(MAX_LEVELS - 1)];
        self.generation += 1;
        let parent = self.item_mut(parent)?;
        Some(parent.child_mut_or_insert(last, flags))
    }

    /// `Menu Del` (whole menu, `MnRaz`) or `Menu Del(path)`.
    pub fn delete(&mut self, path: &[u16]) {
        if path.is_empty() {
            self.root = MenuItem::default();
            self.active = false;
            self.screen = None;
        } else if let Some(parent) = self.item_mut(&path[..path.len() - 1]) {
            parent.children.retain(|c| c.number != path[path.len() - 1]);
        }
        self.generation += 1;
    }

    /// Records a choice (from the mouse or a shortcut).
    pub fn choose(&mut self, path: &[u16]) {
        self.choice = [0; MAX_LEVELS];
        for (i, &n) in path.iter().take(MAX_LEVELS).enumerate() {
            self.choice[i] = n;
        }
        self.choice_pending = true;
    }

    /// Finds the leaf whose shortcut matches a key press
    /// (`MenuKeyExplore`). Returns its path.
    pub fn find_key(&self, ascii: u8, scan: u8, shift: u8) -> Option<Vec<u16>> {
        fn walk(
            item: &MenuItem,
            path: &mut Vec<u16>,
            ascii: u8,
            scan: u8,
            shift: u8,
        ) -> Option<Vec<u16>> {
            for c in &item.children {
                if c.flags & flags::INACTIVE != 0 {
                    continue;
                }
                path.push(c.number);
                let hit = match c.key {
                    Some(MenuKey::Ascii(a)) => ascii != 0 && a.eq_ignore_ascii_case(&ascii),
                    Some(MenuKey::Scancode { scan: s, shift: sh }) => {
                        s == scan && (sh == 0 || shift & sh != 0)
                    }
                    None => false,
                };
                if hit && c.children.is_empty() {
                    return Some(path.clone());
                }
                if let Some(p) = walk(c, path, ascii, scan, shift) {
                    return Some(p);
                }
                path.pop();
            }
            None
        }
        walk(&self.root, &mut Vec::new(), ascii, scan, shift)
    }

    /// `Menu To Bank`: the tree in the original bank format.
    pub fn to_bank(&self) -> Option<Vec<u8>> {
        if self.root.children.is_empty() {
            return None;
        }
        let mut out = Vec::new();
        write_list(&self.root.children, &mut out);
        Some(out)
    }

    /// `Bank To Menu`: rebuilds the tree from a "Menu    " bank. Returns
    /// false if the data is not a valid menu tree.
    pub fn from_bank(&mut self, data: &[u8]) -> bool {
        let mut budget = data.len() / NODE_LEN + 1;
        let Some(list) = read_list(data, 0, &mut budget, 0) else {
            return false;
        };
        self.root = MenuItem {
            children: list,
            ..Default::default()
        };
        self.generation += 1;
        true
    }
}

// ---------------------------------------------------------------------
// Bank format (`InMenuToBank` / `InBankToMenu`, +Lib.s:15372-15560)
// ---------------------------------------------------------------------

/// Size of a menu node (`MnLong`).
const NODE_LEN: usize = 70;

/// Opcode of a text object (`MnOPr`); commands are 8 + 4 * index.
const OP_TEXT: u16 = 4;

/// Compiled object bytes (`MnObjet`): total size word, opcodes and their
/// word arguments, a 0 word at the end.
pub fn encode_ops(ops: &[MenuOp]) -> Vec<u8> {
    let mut b: Vec<u8> = vec![0, 0];
    let w = |b: &mut Vec<u8>, v: i32| b.extend_from_slice(&(v as u16).to_be_bytes());
    for op in ops {
        let (idx, args): (usize, [i16; 2]) = match op {
            MenuOp::Text(t) => {
                w(&mut b, OP_TEXT as i32);
                w(&mut b, t.len() as i32);
                b.extend_from_slice(t);
                if t.len() & 1 != 0 {
                    b.push(0);
                }
                continue;
            }
            MenuOp::Proc(name) => {
                w(&mut b, 8 + 4 * 11);
                let len = (name.len() + 1) & !1;
                w(&mut b, len as i32);
                b.extend_from_slice(name);
                b.resize(b.len() + len - name.len(), 0);
                continue;
            }
            MenuOp::Bar(x, y) => (0, [*x, *y]),
            MenuOp::Line(x, y) => (1, [*x, *y]),
            MenuOp::Ellipse(x, y) => (2, [*x, *y]),
            MenuOp::Pattern(n) => (3, [*n, 0]),
            MenuOp::Ink(n, c) => (4, [*n, *c]),
            MenuOp::Bob(n) => (5, [*n, 0]),
            MenuOp::Icon(n) => (6, [*n, 0]),
            MenuOp::Locate(x, y) => (7, [*x, *y]),
            MenuOp::Outline(n) => (8, [*n, 0]),
            MenuOp::SetLine(n) => (9, [*n, 0]),
            MenuOp::SetFont(n) => (10, [*n, 0]),
            MenuOp::Reserve(n) => (12, [*n, 0]),
            MenuOp::Style(n) => (13, [*n, 0]),
        };
        w(&mut b, 8 + 4 * idx as i32);
        for a in args.iter().take(COMMANDS[idx].1 as usize) {
            w(&mut b, *a as i32);
        }
    }
    w(&mut b, 0);
    let len = b.len() as u16;
    b[0..2].copy_from_slice(&len.to_be_bytes());
    b
}

/// Decodes a compiled object (the inverse of [`encode_ops`]).
pub fn decode_ops(d: &[u8]) -> Option<Vec<MenuOp>> {
    let rd = |p: usize| d.get(p..p + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let size = (rd(0)? as usize).min(d.len());
    let mut p = 2;
    let mut ops = Vec::new();
    while p + 2 <= size {
        let op = rd(p)?;
        p += 2;
        match op {
            0 => break,
            OP_TEXT => {
                let len = rd(p)? as usize;
                let t = d.get(p + 2..p + 2 + len)?;
                ops.push(MenuOp::Text(t.to_vec()));
                p += 2 + ((len + 1) & !1);
            }
            _ if op >= 8 && op % 4 == 0 && ((op - 8) / 4) < COMMANDS.len() as u16 => {
                let idx = ((op - 8) / 4) as usize;
                if idx == 11 {
                    let len = rd(p)? as usize;
                    let name = d.get(p + 2..p + 2 + len)?;
                    let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
                    ops.push(MenuOp::Proc(name[..end].to_vec()));
                    p += 2 + len;
                    continue;
                }
                let n = COMMANDS[idx].1 as usize;
                let a = rd(p)? as i16;
                let b = if n > 1 { rd(p + 2)? as i16 } else { 0 };
                p += 2 * n;
                ops.push(match idx {
                    0 => MenuOp::Bar(a, b),
                    1 => MenuOp::Line(a, b),
                    2 => MenuOp::Ellipse(a, b),
                    3 => MenuOp::Pattern(a),
                    4 => MenuOp::Ink(a, b),
                    5 => MenuOp::Bob(a),
                    6 => MenuOp::Icon(a),
                    7 => MenuOp::Locate(a, b),
                    8 => MenuOp::Outline(a),
                    9 => MenuOp::SetLine(a),
                    10 => MenuOp::SetFont(a),
                    12 => MenuOp::Reserve(a),
                    _ => MenuOp::Style(a),
                });
            }
            _ => return None,
        }
    }
    Some(ops)
}

/// Writes a list of siblings depth first (`MnTb`): each node, its objects
/// (background, normal, selected, inactive), its sub-menu, then the next
/// sibling. Pointers become offsets from the start of the data.
fn write_list(list: &[MenuItem], out: &mut Vec<u8>) {
    for (i, item) in list.iter().enumerate() {
        let node = out.len();
        out.resize(node + NODE_LEN, 0);
        let put16 = |out: &mut Vec<u8>, o: usize, v: i32| {
            out[node + o..node + o + 2].copy_from_slice(&(v as u16).to_be_bytes())
        };
        let put32 = |out: &mut Vec<u8>, o: usize, v: usize| {
            out[node + o..node + o + 4].copy_from_slice(&(v as u32).to_be_bytes())
        };
        put16(out, 12, item.number as i32);
        // MnFlag: the flag bits in the high byte (bset on the word's
        // address), the Called flag in the low byte. MnFlat marks the first
        // item of a list.
        let mut f = (item.flags & 0xFF) & !flags::FLAT;
        if i == 0 {
            f |= flags::FLAT;
        }
        let called = if item.flags & flags::CALLED != 0 {
            0xFF
        } else {
            0
        };
        put16(out, 14, ((f as i32) << 8) | called);
        for (k, v) in [
            item.x, item.y, item.tx, item.ty, item.mx, item.my, item.xx, item.yy,
        ]
        .into_iter()
        .enumerate()
        {
            put16(out, 16 + 2 * k, v);
        }
        let (kf, ka, ks, kh) = match item.key {
            None => (0u8, 0u8, 0u8, 0u8),
            Some(MenuKey::Ascii(a)) => (1, a, 0, 0),
            Some(MenuKey::Scancode { scan, shift }) => (0xFF, 0, scan, shift),
        };
        out[node + 34..node + 38].copy_from_slice(&[kf, ka, ks, kh]);
        out[node + 64..node + 70].copy_from_slice(&item.inks);
        // Objects in the order ObF, Ob1, Ob2, Ob3 (offsets 38, 42, 46, 50).
        for (slot, obj) in [(38, 3), (42, 0), (46, 1), (50, 2)] {
            if let Some(ops) = &item.objects[obj] {
                let at = out.len();
                put32(out, slot, at);
                out.extend_from_slice(&encode_ops(ops));
            }
        }
        if !item.children.is_empty() {
            let at = out.len();
            put32(out, 8, at);
            write_list(&item.children, out);
        }
        if i + 1 < list.len() {
            let at = out.len();
            put32(out, 4, at);
        }
    }
}

/// Reads a list of siblings starting at `off` (`MnTMn`).
fn read_list(d: &[u8], mut off: usize, budget: &mut usize, depth: usize) -> Option<Vec<MenuItem>> {
    let mut list = Vec::new();
    loop {
        if *budget == 0 || depth >= MAX_LEVELS || off + NODE_LEN > d.len() {
            return None;
        }
        *budget -= 1;
        let n = &d[off..off + NODE_LEN];
        let r16 = |o: usize| u16::from_be_bytes([n[o], n[o + 1]]);
        let r32 = |o: usize| u32::from_be_bytes([n[o], n[o + 1], n[o + 2], n[o + 3]]) as usize;
        let s16 = |o: usize| r16(o) as i16 as i32;
        let flag = r16(14);
        let mut flags = (flag >> 8) & 0xFF;
        if flag & 0xFF != 0 {
            flags |= flags::CALLED;
        }
        let key = match n[34] {
            0 => None,
            1 => Some(MenuKey::Ascii(n[35])),
            _ => Some(MenuKey::Scancode {
                scan: n[36],
                shift: n[37],
            }),
        };
        let mut objects: [Option<Vec<MenuOp>>; 4] = Default::default();
        for (slot, obj) in [(38, 3), (42, 0), (46, 1), (50, 2)] {
            let o = r32(slot);
            if o != 0 {
                objects[obj] = Some(decode_ops(d.get(o..)?)?);
            }
        }
        let mut inks = [0u8; 6];
        inks.copy_from_slice(&n[64..70]);
        let lat = r32(8);
        let children = if lat != 0 {
            read_list(d, lat, budget, depth + 1)?
        } else {
            Vec::new()
        };
        list.push(MenuItem {
            number: r16(12),
            flags,
            x: s16(16),
            y: s16(18),
            tx: s16(20),
            ty: s16(22),
            mx: s16(24),
            my: s16(26),
            xx: s16(28),
            yy: s16(30),
            objects,
            key,
            inks,
            children,
        });
        let next = r32(4);
        if next == 0 {
            return Some(list);
        }
        off = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_strings() {
        assert_eq!(
            compile(b"Load").unwrap(),
            vec![MenuOp::Text(b"Load".to_vec())]
        );
        assert_eq!(
            compile(b"(BA 20,10)Load(LO 2,1:IN 1,3)").unwrap(),
            vec![
                MenuOp::Bar(20, 10),
                MenuOp::Text(b"Load".to_vec()),
                MenuOp::Locate(2, 1),
                MenuOp::Ink(1, 3)
            ]
        );
        assert_eq!(compile(b"(Bar -5,6)").unwrap(), vec![MenuOp::Bar(-5, 6)]);
        assert_eq!(
            compile(b"(PROC MYDRAW)").unwrap(),
            vec![MenuOp::Proc(b"MYDRAW".to_vec())]
        );
        assert!(compile(b"(XX 1)").is_none());
    }

    #[test]
    fn tree_and_keys() {
        let mut m = Menus::default();
        m.item_create(&[1]).unwrap().objects[0] = compile(b"File");
        m.item_create(&[1, 2]).unwrap().key = Some(MenuKey::Ascii(b'q'));
        m.item_create(&[2]);
        // The parent must exist.
        assert!(m.item_create(&[5, 1]).is_none());
        assert_eq!(m.root.children[0].flags, flags::TLINE | flags::MOVABLE);
        assert_eq!(
            m.root.children[0].children[0].flags,
            flags::BAR | flags::MOVABLE
        );
        assert_eq!(m.root.children.len(), 2);
        assert_eq!(m.find_key(b'Q', 0x10, 0), Some(vec![1, 2]));
        m.delete(&[1]);
        assert_eq!(m.root.children.len(), 1);
    }

    #[test]
    fn objects_encode_like_mnobjet() {
        let ops = compile(b"PICTURE  ").unwrap();
        // Size, text opcode, length, 9 bytes + pad, end word.
        let b = encode_ops(&ops);
        assert_eq!(&b[..6], &[0, 18, 0, 4, 0, 9]);
        assert_eq!(b.len(), 18);
        let ops = compile(b"(IN1,2)(SS4)PICTURE(SS0)(PR DRAW)(BA 3,-4)").unwrap();
        assert_eq!(decode_ops(&encode_ops(&ops)).unwrap(), ops);
    }

    #[test]
    fn bank_round_trip_and_original_file() {
        let mut m = Menus::default();
        m.item_create(&[1]).unwrap().objects[0] = compile(b"File");
        m.item_create(&[1, 1]).unwrap().objects[0] = compile(b"Load");
        let it = m.item_create(&[1, 2]).unwrap();
        it.objects[0] = compile(b"Quit");
        it.objects[3] = compile(b"(BA 40,10)");
        it.key = Some(MenuKey::Scancode {
            scan: 0x45,
            shift: 8,
        });
        it.flags |= flags::CALLED | flags::INACTIVE;
        m.item_create(&[2]).unwrap().objects[0] = compile(b"Edit");
        let bank = m.to_bank().unwrap();
        // Node, its normal object, then the children, then the sibling.
        assert_eq!(
            &bank[12..16],
            &[0, 1, (flags::FLAT | flags::TLINE | flags::MOVABLE) as u8, 0]
        );
        let mut m2 = Menus::default();
        assert!(m2.from_bank(&bank));
        assert_eq!(m2.root.children.len(), 2);
        let q = m2.item(&[1, 2]).unwrap();
        assert_eq!(q.objects[3], compile(b"(BA 40,10)"));
        assert_eq!(
            q.key,
            Some(MenuKey::Scancode {
                scan: 0x45,
                shift: 8
            })
        );
        assert_eq!(
            q.flags & !flags::FLAT,
            flags::BAR | flags::MOVABLE | flags::CALLED | flags::INACTIVE
        );
        assert_eq!(m2.to_bank().unwrap(), bank);
        assert!(!m2.from_bank(&bank[..40]));

        // The bank of the menus tutorial.
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../AMOS-Professional-365/AMOS/Tutorial/Tutorials/Menus/Data.Menu"
        );
        if let Ok(file) = std::fs::read(path) {
            let banks = crate::banks::parse_banks(&file).unwrap();
            let data = banks[0].raw().unwrap();
            let mut m3 = Menus::default();
            assert!(m3.from_bank(data));
            assert_eq!(m3.root.children.len(), 5);
            assert_eq!(m3.item(&[1]).unwrap().objects[0], compile(b"PICTURE  "));
            assert_eq!(m3.to_bank().unwrap().len(), data.len());
        }
    }
}
