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
            let (idx, nargs) = COMMANDS.iter().enumerate().find(|(_, (n, _))| **n == [c1, c2]).map(|(k, (_, a))| (k, *a))?;
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

/// One menu item and its sub-menu.
#[derive(Clone, Debug, Default)]
pub struct MenuItem {
    /// Number of the item at its level (1..1023).
    pub number: u16,
    pub flags: u16,
    /// Position set with `Set Menu` (relative to the parent).
    pub x: i32,
    pub y: i32,
    /// Compiled strings: normal, selected, inactive, background.
    pub objects: [Option<Vec<MenuOp>>; 4],
    pub key: Option<MenuKey>,
    /// Inks of the screen when the item was defined (ink, paper, outline
    /// for normal, then for selected).
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
                self.children.insert(p, MenuItem { number: n, flags, ..Default::default() });
                p
            }
        };
        &mut self.children[pos]
    }
}

/// All menu state of a program.
#[derive(Clone, Debug, Default)]
pub struct Menus {
    /// The menu bar: the root's children are the titles.
    pub root: MenuItem,
    /// Default flags of new items for each level (`Menu Bar 1`...).
    pub default_flags: [u16; MAX_LEVELS],
    /// `Menu On`: the right mouse button opens the menu.
    pub active: bool,
    /// `Menu Mouse On`: the menu appears at the mouse position.
    pub at_mouse: bool,
    /// `Menu Base x,y`.
    pub base: (i32, i32),
    /// Item numbers of the last choice for each level (`Choice(n)`).
    pub choice: [u16; MAX_LEVELS],
    /// A choice was made and not read by `=Choice` yet.
    pub choice_pending: bool,
    /// `On Menu Goto|Gosub|Proc` targets and whether the jump is armed.
    pub on_menu: Option<OnMenu>,
    pub on_menu_armed: bool,
    /// Incremented on every change (to redraw).
    pub generation: u64,
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

    /// Finds or creates the item at `path`.
    pub fn item_create(&mut self, path: &[u16]) -> &mut MenuItem {
        let defaults = self.default_flags;
        let mut it = &mut self.root;
        for (level, &n) in path.iter().enumerate() {
            it = it.child_mut_or_insert(n, defaults[level.min(MAX_LEVELS - 1)]);
        }
        self.generation += 1;
        it
    }

    /// `Menu Del` (whole menu) or `Menu Del(path)`.
    pub fn delete(&mut self, path: &[u16]) {
        if path.is_empty() {
            self.root = MenuItem::default();
            self.active = false;
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
        fn walk(item: &MenuItem, path: &mut Vec<u16>, ascii: u8, scan: u8, shift: u8) -> Option<Vec<u16>> {
            for c in &item.children {
                if c.flags & flags::INACTIVE != 0 {
                    continue;
                }
                path.push(c.number);
                let hit = match c.key {
                    Some(MenuKey::Ascii(a)) => ascii != 0 && a.eq_ignore_ascii_case(&ascii),
                    Some(MenuKey::Scancode { scan: s, shift: sh }) => s == scan && (sh == 0 || shift & sh != 0),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_strings() {
        assert_eq!(compile(b"Load").unwrap(), vec![MenuOp::Text(b"Load".to_vec())]);
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
        assert_eq!(compile(b"(PROC MYDRAW)").unwrap(), vec![MenuOp::Proc(b"MYDRAW".to_vec())]);
        assert!(compile(b"(XX 1)").is_none());
    }

    #[test]
    fn tree_and_keys() {
        let mut m = Menus::default();
        m.item_create(&[1]).objects[0] = compile(b"File");
        m.item_create(&[1, 2]).key = Some(MenuKey::Ascii(b'q'));
        m.item_create(&[2]);
        assert_eq!(m.root.children.len(), 2);
        assert_eq!(m.find_key(b'Q', 0x10, 0), Some(vec![1, 2]));
        m.delete(&[1]);
        assert_eq!(m.root.children.len(), 1);
    }
}
