//! Menu instructions (`+ILib.s:6731-6950`, `+Lib.s:15326-15760`).

use super::Hardware;
use crate::errors;
use crate::interp::value::Value;
use crate::interp::{Ctl, Exc, Interp, R, err};
use crate::menus::{self, MenuKey, OnMenu, OnMenuKind, flags};
use crate::tokens::{Keyword, TK_COMMA, TK_PAR1, TK_PAR2, TK_TO, tk};

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
                let ink = self.screens.current().map(|s| (s.gr.ink, s.gr.paper, s.gr.outline)).unwrap_or((2, 1, 2));
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
                let mut objects: [Option<Option<Vec<menus::MenuOp>>>; 4] = Default::default();
                for (o, s) in objects.iter_mut().zip(&strings) {
                    if let Some(s) = s {
                        *o = Some(if s.is_empty() { None } else { Some(menus::compile(s).ok_or(Exc::Error(errors::SYNTAX_ERROR))?) });
                    }
                }
                let item = self.menus.item_create(&path);
                item.inks = [ink.0, ink.1, ink.2, ink.1, ink.0, ink.2];
                for (i, o) in objects.into_iter().enumerate() {
                    if let Some(o) = o {
                        item.objects[i] = o;
                    }
                }
            }
            MENU_DEL => {
                let path = if it.peek() == TK_PAR1 { self.menu_path(it)? } else { Vec::new() };
                self.menus.delete(&path);
            }
            SET_MENU => {
                let path = self.menu_path(it)?;
                it.expect(TK_TO)?;
                let x = it.eval_int(self)?;
                it.expect(TK_COMMA)?;
                let y = it.eval_int(self)?;
                let item = self.menus.item_mut(&path).ok_or(Exc::Error(39))?;
                item.x = x;
                item.y = y;
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
                            Some(MenuKey::Scancode { scan: scan as u8, shift: shift as u8 })
                        }
                    }
                } else {
                    None
                };
                let item = self.menus.item_mut(&path).ok_or(Exc::Error(39))?;
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
                    targets.push(if kind == OnMenuKind::Proc { it.proc_operand()? } else { it.label_target(self)? });
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
                let a = it.inst_args(self, kw)?;
                self.menus.base = (a.int(0), a.int(1));
            }
            MENU_CALC => self.menus.generation += 1,
            MENU_BAR | MENU_LINE | MENU_TLINE | MENU_MOVABLE | MENU_STATIC | MENU_ITEM_MOVABLE | MENU_ITEM_STATIC
            | MENU_ACTIVE | MENU_INACTIVE | MENU_SEPARATE | MENU_LINK | MENU_CALLED | MENU_ONCE => {
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
                    let item = self.menus.item_mut(&path).ok_or(Exc::Error(39))?;
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
            MENU_TO_BANK | BANK_TO_MENU => {
                it.inst_args(self, kw)?;
                return Err(Exc::Message("Menu banks are not supported yet".into()));
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub(crate) fn menus_function(&mut self, it: &mut Interp, kw: Keyword) -> R<Option<Value>> {
        if kw.slot != 0 {
            return Ok(None);
        }
        use tk::*;
        let v = match kw.token {
            CHOICE => Value::Int(if std::mem::take(&mut self.menus.choice_pending) { -1 } else { 0 }),
            CHOICE_2 => {
                let n = it.func_args(self, kw)?.int(0);
                if !(1..=menus::MAX_LEVELS as i32).contains(&n) {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                Value::Int(self.menus.choice[n as usize - 1] as i32)
            }
            X_MENU | Y_MENU => {
                let path = self.menu_path(it)?;
                let item = self.menus.item(&path).ok_or(Exc::Error(39))?;
                Value::Int(if kw.token == X_MENU { item.x } else { item.y })
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

    /// Menu shortcuts and the On Menu jump, at interpreter test points.
    pub(crate) fn menus_test_point(&mut self, it: &mut Interp) -> R<()> {
        if self.menus.active && self.input.key_serial != self.menu_key_serial {
            self.menu_key_serial = self.input.key_serial;
            let k = self.input.last_key;
            if let Some(path) = self.menus.find_key(k.ascii, k.raw, k.shift) {
                self.menus.choose(&path);
            }
        }
        if !(self.menus.choice_pending && self.menus.on_menu_armed) {
            return Ok(());
        }
        let Some(on) = self.menus.on_menu.clone() else { return Ok(()) };
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

#[cfg(test)]
mod tests {
    use crate::Machine;
    use crate::input::InputEvent;
    use crate::interp::RunState;

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
        m.input(InputEvent::Key { scancode: 0x10, pressed: true, ch: Some('q') });
        for _ in 0..5 {
            m.vbl();
        }
        assert!(matches!(m.state, RunState::Stopped(_)), "{:?}", m.state);
        assert!(m.interp.prg.is_some());
    }
}
