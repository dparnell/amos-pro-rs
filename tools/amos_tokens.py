#!/usr/bin/env python3
"""
AMOS Professional .AMOS file lister / detokeniser.

Builds the token tables by "assembling" the token tables found in the
AMOS Pro 68000 sources (+Lib.s for the main table, extension sources for the
extension tables, +Edit.s Dtk_Operateurs for the operators), then lists a
tokenised .AMOS program as ASCII text, following the algorithm of the
editor's detokeniser (+Edit.s Detok, line 14744).

Usage:
    detok.py [--src DIR] [--libs DIR] [--banks] [--raw] file.AMOS [...]
      --src DIR   AMOS-Professional source dir (default: auto)
      --libs DIR  use binary AMOSPro*.Lib files from DIR instead of sources
      --banks     also list the banks appended to the program
      --raw       print the hex of each tokenised line after the text
      --dump-table  print the token table and exit
      --utf8      output UTF-8 instead of the default ISO-8859-1
"""
import os
import re
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_SRC = os.path.join(os.path.dirname(HERE), "AMOS-Professional-365")

# ---------------------------------------------------------------------------
# Special token values (+Equ.s 1994-2125)
# ---------------------------------------------------------------------------
TK_VAR = 0x0006    # variable
TK_LAB = 0x000C    # label definition (or line number at start of line)
TK_PRO = 0x0012    # procedure call
TK_LGO = 0x0018    # label/line-number reference (goto/gosub/then/else)
TK_BIN = 0x001E    # %binary constant  (long)
TK_CH1 = 0x0026    # "string"          (word len + bytes, padded even)
TK_CH2 = 0x002E    # 'string'
TK_HEX = 0x0036    # $hex constant     (long)
TK_ENT = 0x003E    # decimal integer   (long)
TK_FL = 0x0046     # single float      (long, FFP format)
TK_EXT = 0x004E    # extension token   (.b ext#, .b 0, .w offset)
TK_PAR1 = 0x0074   # "("
TK_FOR = 0x023C
TK_RPT = 0x0250
TK_WHL = 0x0268
TK_DO = 0x027E
TK_EXIF = 0x0290
TK_EXIT = 0x029E
TK_IF = 0x02BE
TK_ELSE = 0x02D0
TK_ON = 0x0316
TK_PROC = 0x0376
TK_ENDP = 0x0390
TK_DATA = 0x0404
TK_REM1 = 0x064A   # Rem
TK_REM2 = 0x0652   # '
TK_ELSI = 0x25A4
TK_EQU = 0x2A40
TK_STRUS = 0x2A64
TK_DFL = 0x2B6A    # double float constant (8 bytes, IEEE double)

# extension slot -> (source file, binary lib)
EXTENSIONS = {
    1: ("+Music.s", "AMOSPro_Music.Lib"),
    2: ("+Compact.s", "AMOSPro_Compact.Lib"),
    3: ("+Request.s", "AMOSPro_Request.Lib"),
    4: ("+3d.s", "AMOSPro_3d.Lib"),
    5: ("+CompExt.s", "AMOSPro_Compiler.Lib"),
    6: ("+IO_Ports.s", "AMOSPro_IOPorts.Lib"),
}


# ---------------------------------------------------------------------------
# Tiny Devpac dc.b / dc.w "assembler"
# ---------------------------------------------------------------------------
def split_operands(s):
    """Return the operand field (stops at first blank outside quotes)."""
    out, q = [], None
    for ch in s:
        if q:
            out.append(ch)
            if ch == q:
                q = None
        else:
            if ch in " \t":
                break
            if ch in "\"'":
                q = ch
            out.append(ch)
    return "".join(out)


def split_items(s):
    items, cur, q = [], [], None
    for ch in s:
        if q:
            cur.append(ch)
            if ch == q:
                q = None
        elif ch in "\"'":
            q = ch
            cur.append(ch)
        elif ch == ",":
            items.append("".join(cur))
            cur = []
        else:
            cur.append(ch)
    items.append("".join(cur))
    return items


TERM_RE = re.compile(r"""\s*([+-]?)\s*("[^"]*"|'[^']*'|\$[0-9A-Fa-f]+|%[01]+|\d+|[A-Za-z_.][\w.]*)""")


def eval_item(item):
    """Evaluate a dc.b item. Returns a list of byte values."""
    item = item.strip()
    if len(item) >= 2 and item[0] == item[-1] and item[0] in "\"'" and \
            item.count(item[0]) == 2:
        return [ord(c) & 0xFF for c in item[1:-1]]
    pos, total = 0, 0
    while pos < len(item):
        m = TERM_RE.match(item, pos)
        if not m:
            raise ValueError("bad expression %r" % item)
        sign, t = m.group(1), m.group(2)
        if t[0] in "\"'":
            v = ord(t[1])
        elif t[0] == "$":
            v = int(t[1:], 16)
        elif t[0] == "%":
            v = int(t[1:], 2)
        elif t[0].isdigit():
            v = int(t)
        else:
            v = 1  # symbol (L_xxx): value irrelevant, but must be non-zero
        total += -v if sign == "-" else v
        pos = m.end()
    return [total & 0xFF]


def assemble_token_table(path, start_label_re):
    """Assemble the dc.w/dc.b lines of a token table into bytes.
    Returns (bytes, {offset: source_line_number})."""
    with open(path, "rb") as f:
        lines = f.read().decode("latin-1").splitlines()
    out = bytearray()
    srcline = {}
    started = False
    for no, raw in enumerate(lines, 1):
        line = raw.rstrip()
        if not started:
            if re.match(start_label_re, line):
                started = True
            else:
                continue
        if not line.strip() or line.lstrip().startswith((";", "*")):
            continue
        # remove label
        if line[0] not in " \t":
            parts = line.split(None, 1)
            if len(parts) == 1:
                continue
            line = parts[1]
        line = line.strip()
        m = re.match(r"(?i)dc\.([bw])\s+(.*)$", line)
        if not m:
            if line.lower().startswith("even"):
                if len(out) & 1:
                    out.append(0)
            continue
        size, ops = m.group(1).lower(), split_operands(m.group(2))
        items = split_items(ops)
        if size == "w":
            if len(out) & 1:
                out.append(0)          # Devpac aligns dc.w on even address
            if len(items) == 1 and items[0].strip() == "0":
                out += b"\0\0"
                break                   # end of table
            srcline[len(out)] = no
            for it in items:
                v = eval_item(it)[0] if not re.match(r"^\s*-?\d+\s*$", it) else int(it)
                out += struct.pack(">H", v & 0xFFFF)
        else:
            for it in items:
                out += bytes(eval_item(it))
    return bytes(out), srcline


def load_binary_lib(path):
    """Return the token table bytes of an assembled AMOSPro .Lib file."""
    data = open(path, "rb").read()
    assert struct.unpack(">L", data[0:4])[0] == 0x3F3
    nhunks = struct.unpack(">L", data[8:12])[0]
    p = 20 + 4 * nhunks
    assert struct.unpack(">L", data[p:p + 4])[0] & 0x3FFFFFFF == 0x3E9
    code = data[p + 8:]
    tk_minus_off = struct.unpack(">L", code[0:4])[0]
    coff = 22 if code[18:22] == b"AP20" else 18
    return code[coff + tk_minus_off:]


class Entry:
    __slots__ = ("offset", "inst", "func", "name", "params", "term", "line")

    def __repr__(self):
        return "%04X %-24r %-12s %d" % (self.offset, self.name, self.params, self.term)


def walk_table(tb, srcline=None):
    """Walk a token table exactly like +B.s 2333-2358 / TklNext (+Edit.s 14727).
    Returns {offset: Entry}. Name of '$80' entries resolved to previous '!'."""
    entries = {}
    p = 0
    last_bang = None
    while p + 4 <= len(tb):
        inst, func = struct.unpack(">HH", tb[p:p + 4])
        if p > 0 and inst == 0:   # end of table (dc.w 0)
            break
        e = Entry()
        e.offset, e.inst, e.func = p, inst, func
        e.line = (srcline or {}).get(p)
        q = p + 4
        name = bytearray()
        while True:
            b = tb[q]
            q += 1
            name.append(b & 0x7F)
            if b & 0x80:
                break
        rawname = name
        params = bytearray()
        while True:
            b = tb[q]
            q += 1
            if b & 0x80:
                e.term = b - 256
                break
            params.append(b)
        if q & 1:
            q += 1
        if len(rawname) == 1 and rawname[0] == 0:  # name is $80 alone
            e.name = last_bang if last_bang is not None else ""
        else:
            nm = rawname.decode("latin-1")
            if nm.startswith("!"):
                nm = nm[1:]
                last_bang = nm
            e.name = nm
        e.params = params.decode("latin-1")
        entries[p] = e
        p = q
    return entries


def operator_table(edit_src):
    """+Edit.s Dtk_Operateurs (15183): 'bra' (4 bytes) + dc.b, token is the
    negative offset of the entry relative to Dtk_OpFin."""
    with open(edit_src, "rb") as f:
        lines = f.read().decode("latin-1").splitlines()
    i = next(n for n, l in enumerate(lines) if l.startswith("Dtk_Operateurs"))
    blobs = []
    for l in lines[i + 1:]:
        if l.startswith("Dtk_OpFin"):
            break
        m = re.match(r"\s+dc\.b\s+(.*)$", l)
        if m:
            b = bytearray()
            for it in split_items(split_operands(m.group(1))):
                b += bytes(eval_item(it))
            blobs.append(b)
    ops = {}
    sizes = [4 + len(b) + (len(b) & 1) for b in blobs]
    off = -sum(sizes)
    for b, s in zip(blobs, sizes):
        e = Entry()
        e.offset, e.inst, e.func, e.line = off & 0xFFFF, 0, 0, None
        n = 0
        while not b[n] & 0x80:
            n += 1
        e.name = bytes(c & 0x7F for c in b[:n + 1]).decode()
        e.params = bytes(b[n + 1:-1]).decode()
        e.term = -1
        ops[off & 0xFFFF] = e
        off += s
    return ops


class Tables:
    def __init__(self, src=DEFAULT_SRC, libs=None):
        self.ext = {}
        if libs:
            self.main = walk_table(load_binary_lib(os.path.join(libs, "AMOSPro.Lib")))
            for n, (_, lib) in EXTENSIONS.items():
                p = os.path.join(libs, lib)
                if os.path.exists(p):
                    self.ext[n] = walk_table(load_binary_lib(p))
        else:
            tb, sl = assemble_token_table(os.path.join(src, "+Lib.s"), r"^C_Tk\b")
            self.main = walk_table(tb, sl)
            for n, (s, _) in EXTENSIONS.items():
                lab = r"^TokenTable" if s == "+3d.s" else r"^C_Tk\b"
                tb, sl = assemble_token_table(os.path.join(src, s), lab)
                self.ext[n] = walk_table(tb, sl)
        self.ops = operator_table(os.path.join(src, "+Edit.s"))


# ---------------------------------------------------------------------------
# Number formatting
# ---------------------------------------------------------------------------
def ffp_to_float(v):
    """Motorola Fast Floating Point: mmmmmmmm mmmmmmmm mmmmmmmm seeeeeee"""
    if v & 0xFFFFFF00 == 0:
        return 0.0
    mant = (v >> 8) & 0xFFFFFF
    exp = (v & 0x7F) - 64
    val = mant / float(1 << 24) * (2.0 ** exp)
    return -val if v & 0x80 else val


def fmt_float(x, digits=7):
    """Approximates L_FloatToAsc (+Lib.s 25922) followed by the '.0' rule
    of DtkC8 (+Edit.s 15073)."""
    s = "%.*G" % (digits, x)
    if "E" in s:
        mant, ex = s.split("E")
        if "." in mant:
            mant = mant.rstrip("0").rstrip(".")
        sign = "-" if ex.startswith("-") else "+"
        # FloatToAsc p5 (+Lib.s ~25966) prints a space before the "E"
        s = "%s E%s%02d" % (mant, sign, abs(int(ex)))
    elif "." in s:
        s = s.rstrip("0").rstrip(".")
    if "." not in s and "E" not in s:
        s += ".0"
    return s


def s32(v):
    return v - (1 << 32) if v & 0x80000000 else v


# ---------------------------------------------------------------------------
# Detokeniser (+Edit.s 14744 Detok)
# ---------------------------------------------------------------------------
def capitalise(name):
    """DtkMaj1=2 (+Editor_Config.s 43): first letter of every word upper."""
    # Dtk8/Dtk9a: char 0 is upper-cased; afterwards a char that follows a
    # space is upper-cased, but char 0 itself is never tested for space
    # (so " xor " stays lower case).
    out = []
    for i, c in enumerate(name):
        up = i == 0 or (i >= 2 and name[i - 1] == " ")
        out.append(c.upper() if up else c)
    return "".join(out)


def detok_line(line, tb, decrypt_hook=None):
    """line: the bytes of one tokenised line (starting at the length byte)."""
    out = []
    indent = line[1]
    if indent >= 2:
        out.append(" " * (indent - 1))
    p = 2
    after_var = False        # d5 bit 0

    def last():
        return out[-1][-1] if out and out[-1] else ""

    def at_start():
        return not "".join(out)

    def rd16(q):
        return struct.unpack(">H", line[q:q + 2])[0]

    while p + 2 <= len(line):
        tk = rd16(p)
        p += 2
        if tk == 0:
            break
        # --- variables / labels / proc calls ------------------------------
        if tk <= TK_LGO:
            if after_var and last() != " ":
                out.append(" ")
            ln, flag = line[p + 2], line[p + 3]
            name = line[p + 4:p + 4 + ln].split(b"\0")[0].decode("latin-1").upper()
            p += 4 + ln
            out.append(name)
            after_var = True
            if tk == TK_LAB:
                if not name[:1].isdigit():
                    out.append(":")
            elif flag & 3 == 1:
                out.append("#")
            elif flag & 3 == 2:
                out.append("$")
            continue
        # --- constants ----------------------------------------------------
        if tk < TK_EXT or tk == TK_DFL:
            if after_var and last() != " ":
                out.append(" ")
            after_var = False
            if tk in (TK_CH1, TK_CH2):
                q = '"' if tk == TK_CH1 else "'"
                n = rd16(p)
                s = line[p + 2:p + 2 + n].decode("latin-1")
                p += 2 + n + (n & 1)
                out.append(q + s + q)
            elif tk == TK_ENT:
                out.append(str(s32(struct.unpack(">L", line[p:p + 4])[0])))
                p += 4
            elif tk == TK_HEX:
                out.append("$%X" % struct.unpack(">L", line[p:p + 4])[0])
                p += 4
            elif tk == TK_BIN:
                out.append("%" + format(struct.unpack(">L", line[p:p + 4])[0], "b"))
                p += 4
            elif tk == TK_FL:
                out.append(fmt_float(ffp_to_float(struct.unpack(">L", line[p:p + 4])[0])))
                p += 4
            elif tk == TK_DFL:
                out.append(fmt_float(struct.unpack(">d", line[p:p + 8])[0], 15))
                p += 8
            else:
                out.append("{?CST %04X}" % tk)
                p += 4
            continue
        after_var = False
        # --- "(" ----------------------------------------------------------
        if tk == TK_PAR1:
            if out and last() == " ":
                out[-1] = out[-1][:-1]
            out.append("(")
            continue
        # --- keyword lookup ----------------------------------------------
        if tk == TK_EXT:
            extn, off = line[p], rd16(p + 2)
            e = tb.ext.get(extn, {}).get(off)
            if e is None:
                e = Entry()
                e.name, e.params = "Extension %s:%04X " % (chr(ord("A") + extn), off), "I"
        elif tk & 0x8000:
            e = tb.ops.get(tk)
        else:
            e = tb.main.get(tk)
        if e is None:
            out.append("{?TK %04X}" % tk)
            continue
        d3 = e.params[:1]
        func_like = d3 in ("O", "V") or ("0" <= d3 <= "8" and d3 != "")
        if not func_like and not at_start() and last() != " ":
            out.append(" ")
        out.append(capitalise(e.name))
        if tk in (TK_REM1, TK_REM2):
            n = rd16(p)
            txt = line[p + 2:p + 2 + n].split(b"\0")[0].decode("latin-1")
            out.append(txt)
            p += 2 + n + (n & 1)
            continue
        if d3 == "I":
            out.append(" ")
        # skip the extra data of special tokens (TInst, +Edit.s 15104)
        if tk in (TK_FOR, TK_RPT, TK_WHL, TK_DO, TK_IF, TK_ELSE, TK_ELSI, TK_DATA):
            p += 2
        elif tk in (TK_EXIT, TK_EXIF, TK_ON, TK_EXT):
            p += 4
        elif tk == TK_PROC:
            p += 8
        elif TK_EQU <= tk <= TK_STRUS:
            p += 6
    return "".join(out)


# ---------------------------------------------------------------------------
# Locked procedure decoding (+Verif.s 5167 ProCode)
# ---------------------------------------------------------------------------
def procode(src, L):
    """src: bytearray of the whole program, L: offset of the Procedure line.
    XORs every line between Procedure and End Proc in place."""
    a6 = L + 2
    if src[a6 + 8] & 0x10:            # compiled procedure: not coded
        return
    size = struct.unpack(">L", src[a6 + 2:a6 + 6])[0]
    a2 = a6 + 16 + size               # End Proc line + 4
    d5 = ((size << 8) | (size >> 24)) & 0xFFFFFFFF
    d5 = (d5 & 0xFFFFFF00) | src[a6 + 9]
    d4 = 1
    d3 = struct.unpack(">H", src[a6 + 6:a6 + 8])[0]
    a1 = L + src[L] * 2
    while True:
        a0 = a1
        a1 = a0 + src[a0] * 2
        a0 += 4
        if a0 == a2:
            break
        while a0 != a1:
            w = struct.unpack(">H", src[a0:a0 + 2])[0] ^ (d5 & 0xFFFF)
            src[a0:a0 + 2] = struct.pack(">H", w)
            a0 += 2
            d5 = (d5 & 0xFFFF0000) | ((d5 + d4) & 0xFFFF)
            d4 = (d4 + d3) & 0xFFFF
            d5 = ((d5 >> 1) | ((d5 & 1) << 31)) & 0xFFFFFFFF
    src[a6 + 8] ^= 0x20


# ---------------------------------------------------------------------------
# File parsing (+Verif.s 4789 Prg_Load, 4964 Prg_Save; +Lib.s Bnk.*)
# ---------------------------------------------------------------------------
def parse_amos(data):
    head = data[:16]
    if head[:10] == b"AMOS Basic":
        kind = "AMOS 1.3"
    elif head[:8] == b"AMOS Pro":
        kind = "AMOS Pro"
    else:
        raise ValueError("not an AMOS program: %r" % head)
    tested = head[11:12] == b"V"
    mathflags = head[15] if kind == "AMOS Pro" else 0
    srclen = struct.unpack(">L", data[16:20])[0]
    src = bytearray(data[20:20 + srclen])
    rest = data[20 + srclen:]
    return dict(kind=kind, header=head, tested=tested, mathflags=mathflags,
                src=src, rest=rest)


def iter_lines(src):
    p = 0
    while p < len(src):
        n = src[p]
        if n == 0:
            break
        if struct.unpack(">H", src[p + 2:p + 4])[0] == TK_PROC:
            flags = src[p + 10]
            if flags & 0x10:
                # machine-code / compiled procedure (+Verif.s 1574-1582,
                # +Edit.s 8730): Procedure line, then "$0301 @_apml_@ .w"
                # followed by raw 68000 code; the End Proc line starts at
                # L+8+size (size field = distance from L+8 to End Proc).
                # Not listable: emit the proc line + a marker.
                size = struct.unpack(">L", src[p + 4:p + 8])[0]
                yield p, bytes(src[p:p + n * 2])
                yield None, ("' [machine code procedure body: %d bytes]" % (size + 8 - n * 2))
                p = p + 8 + size
                continue
            if flags & 0x40 and flags & 0x20:   # locked + coded
                procode(src, p)
        yield p, bytes(src[p:p + n * 2])
        p += n * 2


def parse_banks(rest):
    banks = []
    if len(rest) < 6 or rest[:4] != b"AmBs":
        return banks
    n = struct.unpack(">H", rest[4:6])[0]
    p = 6
    sp_index = 0
    for _ in range(n):
        tag = rest[p:p + 4]
        if tag == b"AmBk":
            num, fast, ln = struct.unpack(">HHL", rest[p + 4:p + 12])
            size = (ln & 0x0FFFFFFF)
            name = rest[p + 12:p + 20].decode("latin-1")
            banks.append(dict(tag="AmBk", number=num, chip=(fast == 0),
                              data=bool(ln & 0x80000000), name=name.rstrip(),
                              length=size - 8, offset=p))
            p += 12 + size
        elif tag in (b"AmSp", b"AmIc"):
            cnt = struct.unpack(">H", rest[p + 4:p + 6])[0]
            q = p + 6
            for _ in range(cnt):
                w, h, d = struct.unpack(">HHH", rest[q:q + 6])
                q += 10 + w * h * d * 2
            q += 64  # palette
            banks.append(dict(tag=tag.decode(), number=1 if tag == b"AmSp" else 2,
                              count=cnt, offset=p, length=q - p))
            p = q
        else:
            banks.append(dict(tag=repr(tag), offset=p))
            break
    return banks


def list_file(path, tb, show_banks=False, raw=False):
    data = open(path, "rb").read()
    prg = parse_amos(data)
    out = []
    for off, line in iter_lines(prg["src"]):
        if off is None:
            out.append(line)
            continue
        out.append(detok_line(line, tb))
        if raw:
            out.append("    ; " + line.hex())
    if show_banks:
        out.append("")
        out.append("; header=%r tested=%s mathflags=%02X source=%d bytes" % (
            prg["header"], prg["tested"], prg["mathflags"], len(prg["src"])))
        for b in parse_banks(prg["rest"]):
            out.append("; bank " + repr(b))
    return "\n".join(out)


def main(argv):
    src, libs, banks, raw, dump = DEFAULT_SRC, None, False, False, False
    if hasattr(sys.stdout, "reconfigure"):
        # AMOS text is Amiga ISO-8859-1; '--utf8' to re-encode for terminals
        enc = "utf-8" if "--utf8" in argv else "latin-1"
        sys.stdout.reconfigure(encoding=enc, errors="replace")
        argv = [a for a in argv if a != "--utf8"]
    files = []
    it = iter(argv)
    for a in it:
        if a == "--src":
            src = next(it)
        elif a == "--libs":
            libs = next(it)
        elif a == "--banks":
            banks = True
        elif a == "--raw":
            raw = True
        elif a == "--dump-table":
            dump = True
        else:
            files.append(a)
    tb = Tables(src, libs)
    if dump:
        for k, e in sorted(tb.ops.items()):
            print("OP  %04X %-10r %s" % (k, e.name, e.params))
        for k, e in sorted(tb.main.items()):
            print("TK  %04X %-26r %-16s %2d  line %s" % (k, e.name, e.params, e.term, e.line))
        for n, t in sorted(tb.ext.items()):
            for k, e in sorted(t.items()):
                print("EX%d %04X %-26r %-16s %2d  line %s" % (n, k, e.name, e.params, e.term, e.line))
        return
    for f in files:
        if len(files) > 1:
            print("=" * 20, f)
        print(list_file(f, tb, banks, raw))


if __name__ == "__main__":
    main(sys.argv[1:])
