//! The editor menu (`EdM_Init`, `EdM_CreObjet`, +Edit.s:12580-13110).
//!
//! The tree comes from `bin/+Editor_Menus.Bin` (8 byte records: function
//! number + '0', two ink digits, a flag, then the position of the item in
//! the tree as four digits - '0'), the texts from `bin/+Editor_Menus.Asc`,
//! both included in the configuration file. Records starting with `*`
//! mark where the original inserts the list of hidden programs.
//!
//! The original builds an ordinary AMOS menu with the Lib's menu system;
//! this module draws an equivalent menu on the editor screen: the right
//! mouse button shows the menu bar, items are chosen by releasing the
//! button over them.

use super::config::EdConfig;
use super::draw;
use crate::gfx::Screen;

#[derive(Clone, Debug, Default)]
pub struct MenuNode {
    /// Position in the parent (the digit of the definition).
    pub number: u8,
    pub text: Vec<u8>,
    /// Editor function (0 = inactive, e.g. separators; < 0 = title).
    pub function: i16,
    /// Inks: pen and paper when not selected.
    pub ink_a: u8,
    pub ink_b: u8,
    pub children: Vec<MenuNode>,
}

impl MenuNode {
    fn insert(&mut self, path: &[u8], node: MenuNode) {
        let n = path[0];
        let idx = match self.children.iter().position(|c| c.number == n) {
            Some(i) => i,
            None => {
                let pos = self.children.iter().position(|c| c.number > n).unwrap_or(self.children.len());
                self.children.insert(pos, MenuNode { number: n, ..MenuNode::default() });
                pos
            }
        };
        if path.len() == 1 {
            let children = std::mem::take(&mut self.children[idx].children);
            self.children[idx] = MenuNode { children, ..node };
        } else {
            self.children[idx].insert(&path[1..], node);
        }
    }

    pub fn active(&self) -> bool {
        self.function != 0
    }
}

/// Builds the editor menu tree from the configuration.
pub fn build(cfg: &EdConfig) -> MenuNode {
    let mut root = MenuNode::default();
    let mut texts = cfg.menus.iter();
    for rec in &cfg.menu_defs {
        if rec[0] == b'*' {
            continue;
        }
        let Some(text) = texts.next() else { break };
        if text.is_empty() {
            continue;
        }
        let path: Vec<u8> = rec[4..8].iter().map(|&c| c.wrapping_sub(b'0')).take_while(|&c| c != 0).collect();
        // The template of the hidden programs list is not shown.
        if path.is_empty() || path.starts_with(&[10, 6]) {
            continue;
        }
        let node = MenuNode {
            number: *path.last().unwrap(),
            text: text.clone(),
            function: rec[0] as i16 - b'0' as i16,
            ink_a: rec[2].wrapping_sub(b'0') & 7,
            ink_b: rec[1].wrapping_sub(b'0') & 7,
            children: Vec::new(),
        };
        root.insert(&path, node);
    }
    // Additions of this port: Build Application at the end of the Project
    // menu (where the program is run and tested), and syntax highlighting
    // on / off at the end of the Config menu.
    if let Some(project) = root.children.iter().find(|c| c.number == 2) {
        let n = project.children.last().map_or(1, |c| c.number + 1);
        let sep = project.children.iter().find(|c| !c.active()).cloned();
        let (ink_a, ink_b) = project.children.first().map_or((0, 3), |c| (c.ink_a, c.ink_b));
        if let Some(sep) = sep {
            root.insert(&[2, n], MenuNode { number: n, ..sep });
        }
        let node = MenuNode {
            number: n + 1,
            text: b" Build Application... ".to_vec(),
            function: super::build::BUILD_FUNCTION as i16,
            ink_a,
            ink_b,
            children: Vec::new(),
        };
        root.insert(&[2, n + 1], node);
        // Same width for all the items of the menu.
        if let Some(project) = root.children.iter_mut().find(|c| c.number == 2) {
            let w = project.children.iter().map(|c| c.text.len()).max().unwrap_or(0);
            for c in &mut project.children {
                let pad = if c.active() { b' ' } else { b'-' };
                c.text.resize(w, pad);
            }
        }
    }
    if let Some(config) = root.children.iter().find(|c| c.number == 6) {
        let n = config.children.last().map_or(1, |c| c.number + 1);
        let (ink_a, ink_b) = config.children.first().map_or((0, 3), |c| (c.ink_a, c.ink_b));
        let width = config.children.first().map_or(21, |c| c.text.len());
        let mut text = b" Syntax Colours".to_vec();
        text.resize(width.max(text.len() + 1), b' ');
        let node = MenuNode { number: n, text, function: SYNTAX_FUNCTION as i16, ink_a, ink_b, children: Vec::new() };
        root.insert(&[6, n], node);
    }
    root
}

/// Editor function number of the syntax highlighting switch (beyond the
/// original functions).
pub const SYNTAX_FUNCTION: u16 = 1101;

/// Height of a menu line in pixels.
const LINE_H: i32 = 9;

/// A box of the open menu: position and the node shown.
#[derive(Clone, Debug)]
struct MenuBox {
    x: i32,
    y: i32,
    w: i32,
    /// Path to the node whose children are listed.
    path: Vec<usize>,
    horizontal: bool,
}

/// State of the open menu.
#[derive(Debug, Default)]
pub struct MenuUi {
    pub open: bool,
    /// Index of the selected item at each level.
    pub sel: Vec<usize>,
    saved: Option<Vec<u8>>,
    drawn_sel: Option<Vec<usize>>,
}

fn node_at<'a>(root: &'a MenuNode, path: &[usize]) -> Option<&'a MenuNode> {
    let mut n = root;
    for &i in path {
        n = n.children.get(i)?;
    }
    Some(n)
}

fn text_w(t: &[u8]) -> i32 {
    t.len() as i32 * 8
}

impl MenuUi {
    /// Opens the menu bar: the screen is saved to be restored on close.
    pub fn open(&mut self, screen: &Screen) {
        self.open = true;
        self.sel.clear();
        self.saved = Some(screen.logic_ref().to_vec());
        self.drawn_sel = None;
    }

    /// Closes the menu and restores the screen.
    pub fn close(&mut self, screen: &mut Screen) {
        self.open = false;
        if let Some(b) = self.saved.take() {
            screen.logic_mut().copy_from_slice(&b);
        }
    }

    /// The boxes shown for the current selection.
    fn boxes(&self, root: &MenuNode) -> Vec<MenuBox> {
        let bar_w = root.children.iter().map(|c| text_w(&c.text)).sum();
        let mut out = vec![MenuBox { x: 0, y: 0, w: bar_w, path: vec![], horizontal: true }];
        let mut path = Vec::new();
        for (level, &s) in self.sel.iter().enumerate() {
            let parent = &out[level];
            let Some(pn) = node_at(root, &parent.path) else { break };
            let Some(item) = pn.children.get(s) else { break };
            if item.children.is_empty() {
                break;
            }
            let (ix, iy) = if parent.horizontal {
                (parent.x + pn.children[..s].iter().map(|c| text_w(&c.text)).sum::<i32>(), LINE_H + 1)
            } else {
                (parent.x + parent.w + 2, parent.y + s as i32 * LINE_H)
            };
            path.push(s);
            let w = item.children.iter().map(|c| text_w(&c.text)).max().unwrap_or(8);
            out.push(MenuBox { x: ix, y: iy, w, path: path.clone(), horizontal: false });
        }
        out
    }

    /// Updates the selection from the mouse position (screen coordinates).
    pub fn track(&mut self, root: &MenuNode, mx: i32, my: i32) {
        let boxes = self.boxes(root);
        // Deepest box under the mouse.
        for (level, b) in boxes.iter().enumerate().rev() {
            let Some(n) = node_at(root, &b.path) else { continue };
            if b.horizontal {
                if (0..LINE_H + 1).contains(&my) && (b.x..b.x + b.w).contains(&mx) {
                    let mut x = b.x;
                    for (i, c) in n.children.iter().enumerate() {
                        let w = text_w(&c.text);
                        if mx >= x && mx < x + w {
                            self.sel.truncate(level);
                            self.sel.push(i);
                            return;
                        }
                        x += w;
                    }
                }
            } else {
                let h = n.children.len() as i32 * LINE_H;
                if mx >= b.x && mx < b.x + b.w + 2 && my >= b.y && my < b.y + h + 2 {
                    let i = ((my - b.y - 1).max(0) / LINE_H) as usize;
                    self.sel.truncate(level);
                    self.sel.push(i.min(n.children.len() - 1));
                    return;
                }
            }
        }
        // Outside every box: the open menus stay, no item is selected.
        if self.sel.len() > 1 && node_at(root, &self.sel).is_some_and(|n| n.children.is_empty()) {
            self.sel.pop();
        }
    }

    /// The function of the selected item, if it is an active leaf.
    pub fn chosen(&self, root: &MenuNode) -> Option<u16> {
        let n = node_at(root, &self.sel)?;
        if self.sel.len() < 2 || !n.children.is_empty() || n.function <= 0 {
            return None;
        }
        Some(n.function as u16)
    }

    /// Draws the menu if the selection changed.
    pub fn draw(&mut self, root: &MenuNode, screen: &mut Screen) {
        if self.drawn_sel.as_ref() == Some(&self.sel) {
            return;
        }
        self.drawn_sel = Some(self.sel.clone());
        if let Some(b) = &self.saved {
            screen.logic_mut().copy_from_slice(b);
        }
        let boxes = self.boxes(root);
        for (level, b) in boxes.iter().enumerate() {
            let Some(n) = node_at(root, &b.path) else { continue };
            let sel = self.sel.get(level).copied();
            if b.horizontal {
                draw::fill(screen, 0, 0, screen.width as i32, LINE_H + 1, 3);
                let mut x = b.x;
                for (i, c) in n.children.iter().enumerate() {
                    let (pen, paper) = if sel == Some(i) { (c.ink_b, c.ink_a) } else { (c.ink_a, c.ink_b) };
                    draw::text(screen, x, 1, &c.text, pen, paper);
                    x += text_w(&c.text);
                }
                draw::fill(screen, 0, LINE_H + 1, screen.width as i32, LINE_H + 2, 0);
            } else {
                let h = n.children.len() as i32 * LINE_H;
                draw::fill(screen, b.x, b.y, b.x + b.w + 2, b.y + h + 2, 3);
                draw::frame(screen, b.x, b.y, b.x + b.w + 1, b.y + h + 1, 0);
                for (i, c) in n.children.iter().enumerate() {
                    let y = b.y + 1 + i as i32 * LINE_H;
                    let hl = sel == Some(i) && c.active();
                    let (pen, paper) = if hl { (c.ink_b, c.ink_a) } else { (c.ink_a, c.ink_b) };
                    if hl {
                        draw::fill(screen, b.x + 1, y, b.x + 1 + b.w, y + LINE_H, paper);
                    }
                    draw::text(screen, b.x + 1, y, &c.text, pen, paper);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_tree() {
        let cfg = EdConfig::defaults();
        let root = build(&cfg);
        let titles: Vec<String> =
            root.children.iter().map(|c| crate::detok::latin1_to_string(&c.text).trim().to_string()).collect();
        assert_eq!(titles, ["Project", "Editor", "Block", "Search", "Config", "User", "Help", "AMOS"]);
        let project = &root.children[0];
        assert_eq!(project.children[0].function, 77);
        assert_eq!(project.children[4].function, 0);
        let mut ui = MenuUi { sel: vec![0, 0], ..Default::default() };
        assert_eq!(ui.chosen(&root), Some(77));
        ui.sel = vec![1, 0];
        assert_eq!(ui.chosen(&root), None);
        ui.sel = vec![1, 0, 0];
        assert_eq!(ui.chosen(&root), Some(87));
        // Tracking: the bar, then the first item of Project.
        ui.sel.clear();
        ui.track(&root, 10, 4);
        assert_eq!(ui.sel, [0]);
        ui.track(&root, 20, LINE_H + 3);
        assert_eq!(ui.sel, [0, 0]);
    }
}
