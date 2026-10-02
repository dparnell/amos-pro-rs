// The imports of a compiled module (`amos_compiler::codegen::IMPORTS`),
// forwarded to `amos_core::compiled::Runtime`. Included by each backend,
// which defines `def!` (module, name, closure over the runtime, the machine
// and the module memory).
{
    def!(l, "host" "enter" |rt, env, mem| -> i32 { rt.enter(env, mem) });
    def!(l, "host" "suspend" |rt, env, mem, p: i32| { rt.suspend(env, p) });
    def!(l, "host" "end_program" |rt, env, mem, last: i32| -> i32 { rt.end_program(env, last) });
    def!(l, "host" "raise" |rt, env, mem, p: i32, c: i32| -> i32 { rt.raise(env, mem, p, c) });
    def!(l, "host" "test_point" |rt, env, mem, p: i32| -> i32 { rt.test_point(env, mem, p) });
    def!(l, "host" "interp" |rt, env, mem, p: i32| -> i32 { rt.interp(env, mem, p) });
    def!(l, "host" "keyword" |rt, env, mem, p: i32| -> i32 { rt.keyword(env, mem, p) });
    def!(l, "host" "fn_i" |rt, env, mem, p: i32, f: i32| -> i32 { rt.fn_i(env, mem, p, f) });
    def!(l, "host" "fn_f" |rt, env, mem, p: i32, f: i32| -> f64 { rt.fn_f(env, mem, p, f) });
    def!(l, "host" "fn_n" |rt, env, mem, p: i32, f: i32| -> f64 { rt.fn_n(env, mem, p, f) });
    def!(l, "host" "fn_s" |rt, env, mem, p: i32, f: i32| -> i32 { rt.fn_s(env, mem, p, f) });
    def!(l, "host" "push_i" |rt, env, mem, v: i32| { rt.push_i(v) });
    def!(l, "host" "push_f" |rt, env, mem, v: f64| { rt.push_f(v) });
    def!(l, "host" "push_s" |rt, env, mem, v: i32| { rt.push_s(v) });
    def!(l, "host" "push_n" |rt, env, mem, v: f64, t: i32| { rt.push_n(v, t) });
    def!(l, "host" "print_begin" |rt, env, mem| { rt.print_begin() });
    def!(l, "host" "print_i" |rt, env, mem, v: i32| { rt.print_i(v) });
    def!(l, "host" "print_f" |rt, env, mem, v: f64| { rt.print_f(env, v) });
    def!(l, "host" "print_n" |rt, env, mem, v: f64, t: i32| { rt.print_n(env, v, t) });
    def!(l, "host" "print_s" |rt, env, mem, v: i32| { rt.print_s(v) });
    def!(l, "host" "print_tab" |rt, env, mem| { rt.print_tab() });
    def!(l, "host" "print_end" |rt, env, mem, p: i32, nl: i32| -> i32 { rt.print_end(env, mem, p, nl) });
    def!(l, "host" "aref" |rt, env, mem, s: i32, n: i32| -> i32 { rt.aref(env, mem, s, n) });
    def!(l, "host" "aget_i" |rt, env, mem, s: i32, i: i32| -> i32 { rt.aget_i(env, s, i) });
    def!(l, "host" "aget_f" |rt, env, mem, s: i32, i: i32| -> f64 { rt.aget_f(env, s, i) });
    def!(l, "host" "aget_s" |rt, env, mem, s: i32, i: i32| -> i32 { rt.aget_s(env, mem, s, i) });
    def!(l, "host" "aset_i" |rt, env, mem, s: i32, i: i32, v: i32| { rt.aset_i(env, mem, s, i, v) });
    def!(l, "host" "aset_f" |rt, env, mem, s: i32, i: i32, v: f64| { rt.aset_f(env, mem, s, i, v) });
    def!(l, "host" "aset_s" |rt, env, mem, s: i32, i: i32, v: i32| { rt.aset_s(env, mem, s, i, v) });
    def!(l, "host" "for_push" |rt, env, mem, p: i32, s: i32, fl: i32, lim: i32, st: i32, b: i32, x: i32| -> i32 {
        rt.for_push(env, mem, p, s, fl, lim, st, b, x)
    });
    def!(l, "host" "next" |rt, env, mem, p: i32| -> i32 { rt.next(env, mem, p) });
    def!(l, "host" "loop_push" |rt, env, mem, p: i32, k: i32, b: i32, x: i32| -> i32 { rt.loop_push(env, mem, p, k, b, x) });
    def!(l, "host" "while_push" |rt, env, mem, p: i32, b: i32, x: i32| -> i32 { rt.while_push(env, mem, p, b, x) });
    def!(l, "host" "until" |rt, env, mem, p: i32, c: i32| -> i32 { rt.until(env, mem, p, c) });
    def!(l, "host" "wend" |rt, env, mem, p: i32| -> i32 { rt.wend(env, mem, p) });
    def!(l, "host" "loop_end" |rt, env, mem, p: i32| -> i32 { rt.loop_end(env, mem, p) });
    def!(l, "host" "exit" |rt, env, mem, p: i32, n: i32, t: i32| -> i32 { rt.exit(env, mem, p, n, t) });
    def!(l, "host" "goto_pos" |rt, env, mem, p: i32, t: i32| -> i32 { rt.goto_pos(env, mem, p, t) });
    def!(l, "host" "goto_label" |rt, env, mem, p: i32, i: i32| -> i32 { rt.goto_label(env, mem, p, i) });
    def!(l, "host" "gosub_label" |rt, env, mem, p: i32, i: i32, r: i32| -> i32 { rt.gosub_label(env, mem, p, i, r) });
    def!(l, "host" "return" |rt, env, mem, p: i32| -> i32 { rt.return_(env, mem, p) });
    def!(l, "host" "call" |rt, env, mem, p: i32, i: i32, r: i32, n: i32| -> i32 { rt.call(env, mem, p, i, r, n) });
    def!(l, "host" "proc_check" |rt, env, mem, p: i32, pop: i32| -> i32 { rt.proc_check(env, mem, p, pop) });
    def!(l, "host" "set_param_i" |rt, env, mem, v: i32| { rt.set_param_i(env, v) });
    def!(l, "host" "set_param_f" |rt, env, mem, v: f64| { rt.set_param_f(env, v) });
    def!(l, "host" "set_param_s" |rt, env, mem, v: i32| { rt.set_param_s(env, v) });
    def!(l, "host" "proc_return" |rt, env, mem, p: i32| -> i32 { rt.proc_return(env, mem, p) });
    def!(l, "host" "dyn_op" |rt, env, mem, op: i32, a: f64, ta: i32, b: f64, tb: i32| -> f64 {
        rt.dyn_op(env, mem, op, a, ta, b, tb)
    });
    def!(l, "host" "next_done" |rt, env, mem, p: i32| -> i32 { rt.next_done(env, mem, p) });
    def!(l, "host" "str_f" |rt, env, mem, x: f64| -> i32 { rt.str_f(env, mem, x) });
    def!(l, "host" "param_i" |rt, env, mem| -> i32 { rt.param_i(env) });
    def!(l, "host" "param_f" |rt, env, mem| -> f64 { rt.param_f(env) });
    def!(l, "host" "param_s" |rt, env, mem| -> i32 { rt.param_s(env, mem) });
    def!(l, "rt" "str_len" |rt, env, mem, h: i32| -> i32 { rt.str_len(h) });
    def!(l, "rt" "str_asc" |rt, env, mem, h: i32| -> i32 { rt.str_asc(h) });
    def!(l, "rt" "chr" |rt, env, mem, n: i32| -> i32 { rt.chr(mem, n) });
    def!(l, "rt" "left_right" |rt, env, mem, h: i32, n: i32, r: i32| -> i32 { rt.left_right(mem, h, n, r) });
    def!(l, "rt" "mid" |rt, env, mem, h: i32, p: i32, n: i32, hn: i32| -> i32 { rt.mid(mem, h, p, n, hn) });
    def!(l, "rt" "str_i" |rt, env, mem, n: i32| -> i32 { rt.str_i(mem, n) });
    def!(l, "rt" "instr" |rt, env, mem, h: i32, n: i32, s: i32, hs: i32| -> i32 { rt.instr(mem, h, n, s, hs) });
    def!(l, "rt" "change_case" |rt, env, mem, h: i32, lw: i32| -> i32 { rt.change_case(mem, h, lw) });
    def!(l, "rt" "int_f" |rt, env, mem, x: f64| -> f64 { rt.int_f(x) });
    def!(l, "host" "wait" |rt, env, mem, p: i32, n: i32| -> i32 { rt.wait(env, mem, p, n) });
    def!(l, "rt" "str_const" |rt, env, mem, p: i32| -> i32 { rt.str_const(mem, p) });
    def!(l, "rt" "str_concat" |rt, env, mem, a: i32, b: i32| -> i32 { rt.str_concat(mem, a, b) });
    def!(l, "rt" "str_minus" |rt, env, mem, a: i32, b: i32| -> i32 { rt.str_minus(mem, a, b) });
    def!(l, "rt" "str_cmp" |rt, env, mem, a: i32, b: i32| -> i32 { rt.str_cmp(a, b) });
    def!(l, "rt" "i2f" |rt, env, mem, v: i32| -> f64 { rt.i2f(v) });
    def!(l, "rt" "fadd" |rt, env, mem, a: f64, b: f64| -> f64 { rt.fadd(a, b) });
    def!(l, "rt" "fsub" |rt, env, mem, a: f64, b: f64| -> f64 { rt.fsub(a, b) });
    def!(l, "rt" "fmul" |rt, env, mem, a: f64, b: f64| -> f64 { rt.fmul(a, b) });
    def!(l, "rt" "fdiv" |rt, env, mem, a: f64, b: f64| -> f64 { rt.fdiv(a, b) });
    def!(l, "rt" "fcmp" |rt, env, mem, a: f64, b: f64| -> i32 { rt.fcmp(a, b) });
    def!(l, "rt" "pow" |rt, env, mem, a: f64, b: f64| -> f64 { rt.pow(a, b) });
}
