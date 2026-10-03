// The imports of a compiled module (`amos_compiler::codegen::IMPORTS`),
// forwarded to `amos_core::compiled::Runtime`. Included by each backend,
// which defines `def!` (module, name, closure over the runtime, the machine
// and the module memory).
{
    def!(l, "host" "enter" |rt, env, mem| -> i32 { rt.enter(env, mem) });
    def!(l, "host" "suspend" |rt, env, mem, p: i32| { rt.suspend(env, mem, p) });
    def!(l, "host" "while_end" |rt, env, mem, p: i32, x: i32| -> i32 { rt.while_end(env, mem, p, x) });
    def!(l, "host" "end_program" |rt, env, mem, last: i32| -> i32 { rt.end_program(env, last) });
    def!(l, "host" "raise" |rt, env, mem, p: i32, c: i32| -> i32 { rt.raise(env, mem, p, c) });
    def!(l, "host" "test_point" |rt, env, mem, p: i32| -> i32 { rt.test_point(env, mem, p) });
    def!(l, "host" "interp" |rt, env, mem, p: i32| -> i32 { rt.interp(env, mem, p) });
    def!(l, "host" "keyword" |rt, env, mem, p: i32, b: i32| -> i32 { rt.keyword(env, mem, p, b) });
    def!(l, "host" "fn_i" |rt, env, mem, p: i32, f: i32, b: i32| -> i32 { rt.fn_i(env, mem, p, f, b) });
    def!(l, "host" "fn_f" |rt, env, mem, p: i32, f: i32, b: i32| -> f64 { rt.fn_f(env, mem, p, f, b) });
    def!(l, "host" "fn_n" |rt, env, mem, p: i32, f: i32, b: i32| -> f64 { rt.fn_n(env, mem, p, f, b) });
    def!(l, "host" "fn_s" |rt, env, mem, p: i32, f: i32, b: i32| -> i32 { rt.fn_s(env, mem, p, f, b) });
    def!(l, "host" "plain_keyword" |rt, env, mem, p: i32, t: i32, k: i32, b: i32| -> i32 {
        rt.plain_keyword(env, mem, p, t, k, b)
    });
    def!(l, "host" "pfn_i" |rt, env, mem, p: i32, t: i32, k: i32, b: i32| -> i32 { rt.pfn_i(env, mem, p, t, k, b) });
    def!(l, "host" "pfn_f" |rt, env, mem, p: i32, t: i32, k: i32, b: i32| -> f64 { rt.pfn_f(env, mem, p, t, k, b) });
    def!(l, "host" "pfn_n" |rt, env, mem, p: i32, t: i32, k: i32, b: i32| -> f64 { rt.pfn_n(env, mem, p, t, k, b) });
    def!(l, "host" "pfn_s" |rt, env, mem, p: i32, t: i32, k: i32, b: i32| -> i32 { rt.pfn_s(env, mem, p, t, k, b) });
    def!(l, "host" "input_sync" |rt, env, mem| -> i32 { rt.input_sync(env, mem) });
    def!(l, "host" "math" |rt, env, mem, t: i32, x: f64| -> f64 { rt.math(env, t, x) });
    pure!(l, "rt" "math" |t: i32, x: f64| -> f64 { amos_core::compiled::runtime::math(t, x) });
    def!(l, "host" "rnd" |rt, env, mem, n: i32| -> i32 { rt.rnd(env, n) });
    def!(l, "host" "push_i" |rt, env, mem, v: i32| { rt.push_i(v) });
    def!(l, "host" "push_f" |rt, env, mem, v: f64| { rt.push_f(v) });
    def!(l, "host" "push_s" |rt, env, mem, v: i32| { rt.push_s(mem, v) });
    def!(l, "host" "push_n" |rt, env, mem, v: f64, t: i32| { rt.push_n(v, t) });
    def!(l, "host" "print_begin" |rt, env, mem| { rt.print_begin() });
    def!(l, "host" "print_i" |rt, env, mem, v: i32| { rt.print_i(v) });
    def!(l, "host" "print_f" |rt, env, mem, v: f64| { rt.print_f(env, v) });
    def!(l, "host" "print_n" |rt, env, mem, v: f64, t: i32| { rt.print_n(env, v, t) });
    def!(l, "host" "print_s" |rt, env, mem, v: i32| { rt.print_s(mem, v) });
    def!(l, "host" "print_tab" |rt, env, mem| { rt.print_tab() });
    def!(l, "host" "print_slots" |rt, env, mem, p: i32, b: i32, n: i32, nl: i32| -> i32 {
        rt.print_slots(env, mem, p, b, n, nl)
    });
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
    def!(l, "host" "call_proc" |rt, env, mem, p: i32, i: i32, r: i32, n: i32| -> i32 { rt.call_native(env, mem, p, i, r, n) });
    def!(l, "host" "goto_value" |rt, env, mem, p: i32, k: i32, r: i32| -> i32 { rt.goto_value(env, mem, p, k, r) });
    def!(l, "host" "fn_def" |rt, env, mem, p: i32, s: i32| -> i32 { rt.fn_def(env, mem, p, s) });
    def!(l, "host" "restore" |rt, env, mem, p: i32, i: i32| -> i32 { rt.restore(env, mem, p, i) });
    def!(l, "host" "read_data" |rt, env, mem, p: i32, t: i32| { rt.read_data(env, mem, p, t) });
    def!(l, "host" "proc_check" |rt, env, mem, p: i32, pop: i32| -> i32 { rt.proc_check(env, mem, p, pop) });
    def!(l, "host" "proc_end" |rt, env, mem, p: i32| -> i32 { rt.proc_end(env, mem, p) });
    def!(l, "host" "dyn_op" |rt, env, mem, op: i32, a: f64, ta: i32, b: f64, tb: i32| -> f64 {
        rt.dyn_op(env, mem, op, a, ta, b, tb)
    });
    def!(l, "host" "next_done" |rt, env, mem, p: i32| -> i32 { rt.next_done(env, mem, p) });
    def!(l, "host" "param_s" |rt, env, mem| -> i32 { rt.param_s(env, mem) });
    def!(l, "rt" "int_f" |rt, env, mem, x: f64| -> f64 { rt.int_f(x) });
    def!(l, "host" "wait" |rt, env, mem, p: i32, n: i32| -> i32 { rt.wait(env, mem, p, n) });
    def!(l, "host" "match_resident" |rt, env, mem, p: i32, s: i32| -> i32 { rt.match_resident(env, mem, p, s) });
    def!(l, "rt" "str_const" |rt, env, mem, p: i32, i: i32| -> i32 { rt.str_const(mem, p, i) });
    def!(l, "rt" "sort_array" |rt, env, mem, a: i32| { rt.sort_array(mem, a) });
    def!(l, "rt" "match_array" |rt, env, mem, a: i32| -> i32 { rt.match_array(mem, a) });
    def!(l, "rt" "pow" |rt, env, mem, a: f64, b: f64| -> f64 { rt.pow(a, b) });
}
