//! Reading instruction and function parameters.

use super::value::{AStr, ENT_NUL, Value};
use super::{Host, Interp, R, err};
use crate::errors;
use crate::tokens::*;

/// Number of parameters kept without a heap allocation.
const INLINE_ARGS: usize = 6;

/// A list of parameter values that only allocates beyond
/// [`INLINE_ARGS`] values (every instruction and function call reads one).
#[derive(Debug, Default)]
pub struct ArgVec {
    len: usize,
    /// The values while there are at most `INLINE_ARGS`; the slots past
    /// `len` hold `Value::Int(0)`, which needs no dropping (see `Drop`).
    inline: std::mem::ManuallyDrop<[Value; INLINE_ARGS]>,
    /// All the values once there are more than `INLINE_ARGS`.
    spill: Vec<Value>,
}

impl Drop for ArgVec {
    #[inline]
    fn drop(&mut self) {
        // Only the slots in use can hold strings (taken values and unused
        // slots are `Value::Int(0)`).
        let n = self.len.min(INLINE_ARGS);
        for v in &mut self.inline[..n] {
            drop(std::mem::take(v));
        }
    }
}

impl ArgVec {
    #[inline(always)]
    pub fn push(&mut self, v: Value) {
        if self.spill.is_empty() && self.len < INLINE_ARGS {
            self.inline[self.len] = v;
            self.len += 1;
            return;
        }
        if self.spill.is_empty() {
            self.spill
                .extend(self.inline.iter_mut().map(std::mem::take));
        }
        self.spill.push(v);
        self.len += 1;
    }

    /// Moves value `i` out (leaving `Value::Int(0)`).
    pub fn take(&mut self, i: usize) -> Value {
        std::mem::take(&mut self[i])
    }
}

impl std::ops::Deref for ArgVec {
    type Target = [Value];
    #[inline]
    fn deref(&self) -> &[Value] {
        if self.spill.is_empty() {
            &self.inline[..self.len]
        } else {
            &self.spill
        }
    }
}

impl std::ops::DerefMut for ArgVec {
    #[inline]
    fn deref_mut(&mut self) -> &mut [Value] {
        if self.spill.is_empty() {
            &mut self.inline[..self.len]
        } else {
            &mut self.spill
        }
    }
}

/// Parameters of an instruction or function, already converted to the
/// types of the signature. Omitted parameters hold [`ENT_NUL`].
#[derive(Debug, Default)]
pub struct Args(pub ArgVec);

impl Args {
    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[inline]
    pub fn int(&self, i: usize) -> i32 {
        match self.0.get(i) {
            Some(Value::Int(v)) => *v,
            Some(Value::Float(f)) => super::value::float_to_int(*f),
            _ => ENT_NUL,
        }
    }

    /// Integer parameter, or `None` when omitted.
    #[inline]
    pub fn opt(&self, i: usize) -> Option<i32> {
        let v = self.int(i);
        if v == ENT_NUL { None } else { Some(v) }
    }

    #[inline]
    pub fn float(&self, i: usize) -> f64 {
        match self.0.get(i) {
            Some(Value::Int(v)) => *v as f64,
            Some(Value::Float(f)) => *f,
            _ => 0.0,
        }
    }

    #[inline]
    pub fn str(&self, i: usize) -> AStr {
        match self.0.get(i) {
            Some(Value::Str(s)) => s.clone(),
            _ => super::value::empty_str(),
        }
    }

    pub fn value(&self, i: usize) -> Value {
        self.0.get(i).cloned().unwrap_or(Value::Int(ENT_NUL))
    }

    pub fn is_str(&self, i: usize) -> bool {
        matches!(self.0.get(i), Some(Value::Str(_)))
    }
}

impl Interp {
    /// Reads parameters following the signature `sig` (type chars separated
    /// by `,` or `t` for `To`), as the patched `Parameters` routines do.
    pub fn args(&mut self, hw: &mut dyn Host, sig: &str) -> R<ArgVec> {
        let mut out = ArgVec::default();
        self.args_into(hw, sig, &mut out)?;
        Ok(out)
    }

    /// `args` into `out` (no copy of the list on return).
    pub fn args_into(&mut self, hw: &mut dyn Host, sig: &str, out: &mut ArgVec) -> R<()> {
        let sig = sig.as_bytes();
        let mut i = 0;
        while i < sig.len() {
            let ty = sig[i];
            let t = self.peek();
            let v = if t == TK_COMMA || t == TK_TO || t == TK_PAR2 || Self::is_end(t) {
                Value::Int(ENT_NUL)
            } else {
                let v = self.eval(hw)?;
                self.convert_param(ty, v)?
            };
            out.push(v);
            if let Some(&sep) = sig.get(i + 1) {
                let want = if sep == b't' { TK_TO } else { TK_COMMA };
                if self.peek() == want {
                    self.pc += 2;
                } else {
                    return err(errors::SYNTAX_ERROR);
                }
            }
            i += 2;
        }
        Ok(())
    }

    /// Reads the parameters of the instruction `kw` (pc after the token),
    /// or takes the ones given to [`Interp::preset_args`].
    pub fn inst_args(&mut self, hw: &mut dyn Host, kw: Keyword) -> R<Args> {
        let sig = param_types_of(kw);
        if self.preset_set {
            return self.take_preset(sig);
        }
        let mut out = Args::default();
        self.args_into(hw, sig, &mut out.0)?;
        Ok(out)
    }

    /// Reads the parameters of the function `kw` (pc after the token), or
    /// takes the ones given to [`Interp::preset_args`].
    pub fn func_args(&mut self, hw: &mut dyn Host, kw: Keyword) -> R<Args> {
        let sig = param_types_of(kw);
        if self.preset_set {
            return self.take_preset(sig);
        }
        let mut out = Args::default();
        if !sig.is_empty() {
            // `fn_args` into `out`.
            self.expect(TK_PAR1)?;
            self.args_into(hw, sig, &mut out.0)?;
            self.expect(TK_PAR2)?;
        }
        Ok(out)
    }

    /// The preset parameters converted for `sig`; the preset is used up
    /// (also on error) and its vector kept for the next one.
    fn take_preset(&mut self, sig: &str) -> R<Args> {
        self.preset_set = false;
        let mut values = std::mem::take(&mut self.preset);
        let mut out = Args::default();
        let r = self.preset_into(sig, &mut values, &mut out.0);
        values.clear();
        self.preset = values;
        r.map(|()| out)
    }

    /// Gives the parameters of the next `inst_args` / `func_args` call,
    /// already evaluated (compiled code calling a keyword handler without a
    /// token stream). `None` is an omitted parameter (`Ink ,2`). The values
    /// are converted to the signature exactly as `args` converts what it
    /// evaluates; the preset is used by the next call only (also when that
    /// call fails). Only for keywords where [`plain_args`] is true and that
    /// have parameters (a handler without parameters may never read them).
    ///
    /// [`plain_args`]: crate::machine::plain_args
    pub fn preset_args(&mut self, args: &[Option<Value>]) {
        self.preset.clear();
        self.preset.extend_from_slice(args);
        self.preset_set = true;
    }

    /// `args(sig)` on values already evaluated: the same conversions and
    /// errors in the same order. Fewer values than the signature is the
    /// missing separator of the token path (Syntax error after the last
    /// value given); more values, the separator left unread, is reported
    /// once the signature's values are converted.
    fn preset_into(&self, sig: &str, values: &mut [Option<Value>], out: &mut ArgVec) -> R<()> {
        let sig = sig.as_bytes();
        let mut i = 0;
        let mut k = 0;
        while i < sig.len() {
            let v = match values.get_mut(k).and_then(Option::take) {
                None => Value::Int(ENT_NUL),
                Some(v) => self.convert_param(sig[i], v)?,
            };
            out.push(v);
            k += 1;
            if sig.get(i + 1).is_some() && k >= values.len() {
                return err(errors::SYNTAX_ERROR);
            }
            i += 2;
        }
        if k < values.len() {
            return err(errors::SYNTAX_ERROR);
        }
        Ok(())
    }

    #[inline]
    pub fn convert_param(&self, ty: u8, v: Value) -> R<Value> {
        // Usual case: already of the type (what the general case returns).
        match (ty, &v) {
            (b'0', Value::Int(_)) | (b'1', Value::Float(_)) | (b'2', Value::Str(_)) => Ok(v),
            _ => self.convert_param_other(ty, v),
        }
    }

    fn convert_param_other(&self, ty: u8, v: Value) -> R<Value> {
        Ok(match ty {
            b'0' => Value::Int(self.to_int(v)?),
            b'1' => Value::Float(self.to_float(v)?),
            b'5' => {
                let mut f = self.to_float(v)?;
                if self.degrees {
                    f = f.to_radians();
                }
                Value::Float(f)
            }
            b'2' => match v {
                s @ Value::Str(_) => s,
                _ => return err(errors::TYPE_MISMATCH),
            },
            _ => v,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::stmt::InputState;
    use super::super::{Exc, RunState};
    use super::*;
    use crate::tokenise::tokenise_program;

    /// Text of a parameter of type `ty` (number `k` of the list) and the
    /// value the token path evaluates it to (before conversion).
    fn param(ty: u8, k: usize, set: usize) -> (String, Value) {
        let ints = [[1, 20, 30, 1, 2, 3, 4, 5], [2, 10, 100, 90, 1, 1, 7, 9]];
        match ty {
            b'2' => ("\"Hi\"".into(), Value::str(b"Hi")),
            // 1.5 and 2.25 are exact in single and double precision.
            b'1' | b'5' => {
                let f = if set == 0 { 1.5 } else { 2.25 };
                (format!("{f}"), Value::Float(f))
            }
            _ => {
                let n = ints[set][k % 8];
                (n.to_string(), Value::Int(n))
            }
        }
    }

    /// Records, for each keyword call, the parameters read from the tokens
    /// and the same values given with `preset_args`.
    #[derive(Default)]
    struct Capture {
        preset: Vec<Option<Value>>,
        seen: Vec<(u16, String, String)>,
    }

    impl Capture {
        fn both(&mut self, it: &mut Interp, kw: Keyword, func: bool) -> R<()> {
            let tokens = if func {
                it.func_args(self, kw)
            } else {
                it.inst_args(self, kw)
            };
            let p = std::mem::take(&mut self.preset);
            it.preset_args(&p);
            let preset = if func {
                it.func_args(self, kw)
            } else {
                it.inst_args(self, kw)
            };
            let show = |r: R<Args>| format!("{:?}", r.map(|a| a.0.to_vec()));
            self.seen.push((kw.token, show(tokens), show(preset)));
            Ok(())
        }
    }

    impl Host for Capture {
        fn instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<()> {
            self.both(it, kw, false)
        }
        fn function(&mut self, it: &mut Interp, kw: Keyword) -> R<Value> {
            self.both(it, kw, true)?;
            Ok(Value::Int(0))
        }
        fn reserved_assign(&mut self, _: &mut Interp, _: Keyword) -> R<()> {
            Ok(())
        }
        fn test_point(&mut self, _: &mut Interp) -> R<()> {
            Ok(())
        }
        fn take_break(&mut self) -> bool {
            false
        }
        fn print(&mut self, _: &mut Interp, _: &[u8]) -> R<()> {
            Ok(())
        }
        fn read_line(&mut self, _: &mut Interp, _: &mut InputState) -> R<Option<Vec<u8>>> {
            Ok(None)
        }
    }

    /// Runs `src` (one call of a keyword with parameters) with the preset
    /// values; returns what the host saw, None if the line is not valid.
    fn capture(src: &str, preset: Vec<Option<Value>>) -> Option<Vec<(u16, String, String)>> {
        let prg = tokenise_program(src.as_bytes()).ok()?;
        let mut it = Interp::new();
        it.load(&prg).ok()?;
        let mut host = Capture {
            preset,
            ..Default::default()
        };
        it.vbl();
        match it.run(&mut host, 1000) {
            RunState::Stopped(_) => Some(host.seen),
            s => panic!("{src}: {s:?}"),
        }
    }

    #[test]
    fn preset_matches_the_token_path_for_plain_keywords() {
        let mut checked = 0;
        for slot_table in [crate::tokens::MAIN] {
            for d in slot_table.iter() {
                let kw = Keyword {
                    slot: 0,
                    token: d.token,
                };
                if !crate::machine::plain_args(kw) {
                    continue;
                }
                let sig = d.param_types().as_bytes();
                if sig.is_empty() {
                    continue;
                }
                let func = !matches!(d.kind(), TokenKind::Instruction);
                let n = sig.len().div_ceil(2);
                let name = d.name.split_whitespace().map(|w| {
                    let mut c = w.chars();
                    c.next()
                        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                        .unwrap_or_default()
                });
                let name: Vec<String> = name.collect();
                let name = name.join(" ");
                // Every parameter given (two value sets), then each one
                // omitted in turn.
                let mut variants: Vec<Vec<Option<usize>>> = vec![(0..n).map(Some).collect()];
                for skip in 0..n {
                    variants.push((0..n).map(|k| (k != skip).then_some(k)).collect());
                }
                for (vi, var) in variants.iter().enumerate() {
                    for set in 0..2 {
                        let mut text = String::new();
                        let mut values = Vec::new();
                        for (k, p) in var.iter().enumerate() {
                            if k > 0 {
                                text += if sig[2 * k - 1] == b't' { " To " } else { "," };
                            }
                            match p {
                                Some(k) => {
                                    let (t, v) = param(sig[2 * k], *k, set);
                                    text += &t;
                                    values.push(Some(v));
                                }
                                None => values.push(None),
                            }
                        }
                        let call = if func {
                            let dollar = if d.params.starts_with('2') { "$" } else { "" };
                            format!("Degree\nA{dollar}={name}({text})\n")
                        } else {
                            format!("Degree\n{name} {text}\n")
                        };
                        let Some(seen) = capture(&call, values) else {
                            assert!(vi > 0, "{call} is not valid");
                            continue;
                        };
                        assert_eq!(seen.len(), 1, "{call}");
                        if seen[0].0 != d.token {
                            // Omitting the only parameter chose another
                            // overload (`Cls`).
                            assert!(vi > 0, "{call}");
                            continue;
                        }
                        assert_eq!(seen[0].1, seen[0].2, "{call}");
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 100, "{checked}");
    }

    impl Interp {
        fn preset_to_args_test(&self, sig: &str, values: &[Option<Value>]) -> R<ArgVec> {
            let mut out = ArgVec::default();
            self.preset_into(sig, &mut values.to_vec(), &mut out)
                .map(|()| out)
        }
    }

    #[test]
    fn preset_conversions_and_errors() {
        let mut it = Interp::new();
        let i = |n| Some(Value::Int(n));
        let f = |x| Some(Value::Float(x));
        let s = || Some(Value::str(b"x"));
        let show = |r: R<ArgVec>| format!("{:?}", r.map(|a| a.to_vec()));
        // Conversions as `convert_param`, omitted values as ENT_NUL.
        assert_eq!(
            show(it.preset_to_args_test("0,1,5,2,3", &[f(2.7), i(3), i(90), s(), None])),
            format!(
                "{:?}",
                Ok::<_, Exc>(vec![
                    Value::Int(2),
                    Value::Float(3.0),
                    Value::Float(90.0),
                    Value::str(b"x"),
                    Value::Int(ENT_NUL)
                ])
            )
        );
        it.degrees = true;
        let r = it.preset_to_args_test("5", &[i(90)]).unwrap();
        assert_eq!(
            format!("{:?}", r[0]),
            format!("{:?}", it.convert_param(b'5', Value::Int(90)).unwrap())
        );
        // Wrong types: the first one met is the error.
        let e = |n: u16| format!("{:?}", Err::<Vec<Value>, _>(Exc::Error(n)));
        assert_eq!(
            show(it.preset_to_args_test("0,2", &[s(), i(1)])),
            e(errors::TYPE_MISMATCH)
        );
        assert_eq!(
            show(it.preset_to_args_test("0,2", &[i(1), i(1)])),
            e(errors::TYPE_MISMATCH)
        );
        // Fewer values: the missing separator (after converting the last
        // value: a type error comes first).
        assert_eq!(
            show(it.preset_to_args_test("0,0", &[i(1)])),
            e(errors::SYNTAX_ERROR)
        );
        assert_eq!(
            show(it.preset_to_args_test("0,0", &[s()])),
            e(errors::TYPE_MISMATCH)
        );
        assert_eq!(
            show(it.preset_to_args_test("0,0", &[])),
            e(errors::SYNTAX_ERROR)
        );
        assert!(it.preset_to_args_test("0", &[]).is_ok());
        // More values than the signature.
        assert_eq!(
            show(it.preset_to_args_test("0", &[i(1), i(2)])),
            e(errors::SYNTAX_ERROR)
        );
        assert_eq!(
            show(it.preset_to_args_test("", &[i(1)])),
            e(errors::SYNTAX_ERROR)
        );
        // The preset is used once, also when it fails.
        let mut host = Capture::default();
        let kw = Keyword {
            slot: 0,
            token: crate::tokens::tk::LOCATE,
        };
        it.preset_args(&[s(), i(1)]);
        assert!(it.inst_args(&mut host, kw).is_err());
        assert!(!it.preset_set);
        it.preset_args(&[i(3), None]);
        let a = it.inst_args(&mut host, kw).unwrap();
        assert_eq!((a.int(0), a.opt(1)), (3, None));
        assert!(!it.preset_set);
    }
}
