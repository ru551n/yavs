#!/usr/bin/env python3
"""Differential fuzzer of cycle mode: random synchronous RTL in the region's subset, with a
seeded testbench printing every output twice a clock, run normal and `--cycle` by one binary.
Every difference is a bug: the designs have no metavalues (every signal initialised, inputs
'0'/'1') and no asserts or reports in the region, the testbench samples between clock edges.
Stdlib only. See README.md "Cycle mode fuzzer".

    cyclefuzz.py gen SEED                   # print the design of SEED
    cyclefuzz.py run [--seeds A:B] [--jobs 2] [--tiers interp,clift] [--warm] [--checks]
    cyclefuzz.py shrink SEED [--tier T]     # reduce a failing SEED to a small reproducer
"""
import argparse, hashlib, json, os, random, re, shutil, subprocess, sys, time
from pathlib import Path

GEN = hashlib.sha1(Path(__file__).read_bytes()).hexdigest()[:10]  # the generator a seed's design came from

WIDTHS = [1, 1, 2, 3, 4, 5, 7, 8, 8, 12, 16, 31, 32, 33, 63, 64, 64, 65, 65, 100, 127, 128, 128, 129]
BIG = 1 << 30


# ---------------------------------------------------------------- types
class T:
    """sl, slv, uns, sgn (w, lo, asc), int (lo, hi), enum (name, lits), rec (name, fields),
    arr (name, elem, lo, hi, asc)."""

    def __init__(s, k, **kw):
        s.k = k
        s.w, s.lo, s.asc, s.hi, s.name, s.lits, s.fields, s.elem = 1, 0, False, 0, None, None, None, None
        s.__dict__.update(kw)
        if k in VEC:
            s.hi = s.lo + s.w - 1

    def mark(s):
        if s.k == "sl":
            return "std_ulogic"
        if s.k in VEC:
            base = {"slv": "std_ulogic_vector", "uns": "unsigned", "sgn": "signed"}[s.k]
            return f"{base}({s.lo} to {s.hi})" if s.asc else f"{base}({s.hi} downto {s.lo})"
        if s.k == "int":
            return f"integer range {s.lo} to {s.hi}"
        return s.name

    def n(s):  # elements of an array, values of an enum
        return s.hi - s.lo + 1 if s.k in ("arr",) + VEC else len(s.lits)

    def same(s, o):
        if s.k != o.k:
            return False
        if s.k in VEC:
            return s.w == o.w
        if s.k == "int":
            return o.lo >= s.lo and o.hi <= s.hi
        return s.k == "sl" or s.name == o.name


VEC = ("slv", "uns", "sgn")


class Ex:
    """An expression: `f` with `{}` holes filled by `a` (each an Ex). `t` its type, `r` an integer
    range, `sig` the signal a name reads."""

    __slots__ = ("f", "a", "t", "r", "sig")

    def __init__(s, f, t, a=(), r=None, sig=None):
        s.f, s.t, s.a, s.r, s.sig = f, t, list(a), r, sig

    def text(s):
        return s.f.format(*[x.text() for x in s.a]) if s.a else s.f

    def walk(s):
        yield s
        for x in s.a:
            yield from x.walk()


class St:
    """A statement: k in assign (tgt Ex, op, rhs Ex), cassign (tgt, op, [(Ex, cond)], else Ex),
    if ([(cond, [St])], else [St] or None), case (sel Ex, [(choice, [St])]), for (var, range, [St]),
    exit/next (cond), call (text, [Ex])."""

    __slots__ = ("k", "a")

    def __init__(s, k, *a):
        s.k, s.a = k, list(a)


def zero(t):
    if t.k == "sl":
        return "'0'"
    if t.k == "uns":
        return f"to_unsigned(0, {t.w})"
    if t.k == "sgn":
        return f"to_signed(0, {t.w})"
    if t.k == "slv":
        return f"std_ulogic_vector(to_unsigned(0, {t.w}))"
    if t.k == "int":
        return str(max(t.lo, min(0, t.hi)))
    if t.k == "enum":
        return t.lits[0]
    if t.k == "rec":
        return f"{t.name}'(" + ", ".join(f"{f} => {zero(ft)}" for f, ft in t.fields) + ")"
    return f"{t.name}'(others => {zero(t.elem)})"


def bits(rng, w):
    return "".join(rng.choice("01") for _ in range(w))


class Obj:
    __slots__ = ("name", "t", "cls", "v", "init")  # cls: sig, var, const, gen, loop

    def __init__(s, name, t, cls):
        s.name, s.t, s.cls, s.v, s.init = name, t, cls, None, None


class Proc:
    """A process or concurrent statement of an entity, rendered by `render_proc`."""

    def __init__(s, k, **kw):
        s.k = k  # clocked, comb, conc, cond, sel, gen_bits, whenclk
        s.label = None
        s.decls, s.body, s.rst_body, s.drives = [], [], [], []
        s.__dict__.update(kw)


class Ent:
    def __init__(s, name):
        s.name = name
        s.generics, s.ports, s.sigs, s.consts, s.procs, s.insts = [], [], [], [], [], []


# ---------------------------------------------------------------- the generator
class Gen:
    def __init__(s, seed, scale=1.0):
        s.rng = random.Random(seed)
        s.seed, s.scale = seed, scale
        s.n = 0
        s.enums, s.recs, s.arrs, s.pconsts, s.funcs, s.procs = [], [], [], [], [], []
        s.ents = []
        s.sigs_read = None

    def fresh(s, p):
        s.n += 1
        return f"{p}{s.n}"

    def chance(s, p):
        return s.rng.random() < p

    def width(s, mx=None):
        w = s.rng.choice(WIDTHS)
        return min(w, mx) if mx else w

    def vtype(s, k=None, w=None):
        k = k or s.rng.choice(VEC)
        w = w or s.width()
        lo = s.rng.choice([0] * 6 + [1, 3, 8])
        return T(k, w=w, lo=lo, asc=s.chance(0.2))

    def scalar_type(s):
        r = s.rng.random()
        if r < 0.2:
            return T("sl")
        if r < 0.7:
            return s.vtype()
        if r < 0.85:
            lo = s.rng.choice([0, 0, -8, -100, 1])
            return T("int", lo=lo, hi=lo + s.rng.choice([1, 7, 15, 100, 255, 1000]))
        return s.rng.choice(s.enums)

    def any_type(s):
        r = s.rng.random()
        if r < 0.12 and s.recs:
            return s.rng.choice(s.recs)
        if r < 0.2 and s.arrs:
            return s.rng.choice(s.arrs)
        return s.scalar_type()

    def make_types(s):
        for i in range(s.rng.randint(1, 3)):
            n = s.rng.randint(2, 6)
            s.enums.append(T("enum", name=f"e{i}_t", lits=[f"E{i}_{j}" for j in range(n)]))
        for i in range(s.rng.randint(0, 3)):
            fs = [(f"f{j}", s.scalar_type()) for j in range(s.rng.randint(1, 4))]
            if s.recs and s.chance(0.3):
                fs.append((f"f{len(fs)}", s.rng.choice(s.recs)))
            s.recs.append(T("rec", name=f"r{i}_t", fields=fs))
        for i in range(s.rng.randint(0, 4)):
            r = s.rng.random()
            el = s.vtype(w=s.width(70)) if r < 0.55 else s.rng.choice(s.recs) if r < 0.7 and s.recs else s.scalar_type()
            if el.k == "arr":
                el = T("sl")
            n = s.rng.choice([1, 2, 4, 4, 8, 16, 5])
            lo = s.rng.choice([0, 0, 0, 1, 4])
            s.arrs.append(T("arr", name=f"a{i}_t", elem=el, lo=lo, hi=lo + n - 1, asc=s.chance(0.8)))
        if s.chance(0.3):  # a bigger memory
            s.arrs.append(T("arr", name=f"a{len(s.arrs)}_t", elem=s.vtype(w=s.width(70)), lo=0, hi=s.rng.choice([31, 63]), asc=True))
        if s.arrs and s.chance(0.3):  # an array of arrays
            el = s.rng.choice(s.arrs)
            s.arrs.append(T("arr", name=f"a{len(s.arrs)}_t", elem=el, lo=0, hi=s.rng.choice([1, 3]), asc=True))

    # ------------------------------------------------------------ literals
    def lit(s, t):
        rng = s.rng
        if t.k == "sl":
            return Ex(rng.choice(["std_ulogic'('0')", "std_ulogic'('1')"]), t)
        if t.k in VEC:
            q = {"slv": "std_ulogic_vector", "uns": "unsigned", "sgn": "signed"}[t.k]
            r = rng.random()
            if r < 0.3 and t.k != "slv":
                if t.k == "uns":
                    v = rng.randrange(1 << min(t.w, 31))
                    return Ex(f"to_unsigned({v}, {t.w})", t)
                h = 1 << (min(t.w, 31) - 1)
                return Ex(f"to_signed({rng.randrange(-h, h)}, {t.w})", t)
            if r < 0.5 and t.w % 4 == 0:
                return Ex(f'{q}\'(x"' + "".join(rng.choice("0123456789ABCDEF") for _ in range(t.w // 4)) + '")', t)
            return Ex(f"{q}'(\"{bits(rng, t.w)}\")", t)
        if t.k == "int":
            return Ex(str(rng.randint(t.lo, t.hi)), t, r=None)
        if t.k == "enum":
            return Ex(rng.choice(t.lits), t)
        return Ex(zero(t), t)

    def const_text(s, t):
        """A literal for a constant or initial value of any type (aggregates in context)."""
        rng = s.rng
        if t.k in VEC:
            if rng.random() < 0.3:
                return f'x"{"".join(rng.choice("0123456789ABCDEF") for _ in range(t.w // 4))}"' if t.w % 4 == 0 else f'"{bits(rng, t.w)}"'
            return f'"{bits(rng, t.w)}"'
        if t.k == "rec":
            return "(" + ", ".join(f"{f} => {s.const_text(ft)}" for f, ft in t.fields) + ")"
        if t.k == "arr":
            if rng.random() < 0.2:
                return f"(others => {s.const_text(t.elem)})"
            return "(" + ", ".join(s.const_text(t.elem) for _ in range(t.n())) + ")" if t.n() > 1 else f"(others => {s.const_text(t.elem)})"
        return s.lit(t).text()

    # ------------------------------------------------------------ names
    def descend(s, o, write=False):
        """A random path into object o: a list of (Ex, type) from the whole object down."""
        e = Ex(o.name, o.t, sig=o.name if o.cls == "sig" else None,
               r=(o.t.lo, o.t.hi) if o.t.k == "int" else None)
        if o.cls == "gen" and o.t.k == "int":
            e.r = o.t.r
        out = [(e, o.t)]
        t = o.t
        for _ in range(3):
            if t.k == "rec" and s.chance(0.8):
                f, ft = s.rng.choice(t.fields)
                e = Ex("{}." + f, ft, [e])
                t = ft
            elif t.k == "arr" and s.chance(0.8):
                ix = s.index(t, write)
                e = Ex("{}({})", t.elem, [e, ix])
                t = t.elem
            else:
                break
            if t.k == "int":
                e.r = (t.lo, t.hi)
            out.append((e, t))
        return out

    def index(s, t, write=False):
        """An index expression into t's range (static or chosen at run time)."""
        n = t.n()
        if n > 1 and s.chance(0.5) and s.depth > 0:
            k = max(1, (n).bit_length() - 1)
            s.depth -= 1
            try:
                u = s.vec("uns", k)
            finally:
                s.depth += 1
            return Ex(f"{t.lo} + to_integer({{}})" if t.lo else "to_integer({})", T("int", lo=t.lo, hi=t.hi), [u])
        return Ex(str(s.rng.randint(t.lo, t.hi)), T("int", lo=t.lo, hi=t.hi))

    def names(s, pred, objs, write=False):
        c = []
        if not write:  # elements, bits, slices and fields of constants too
            objs = objs + s.pconsts + ([] if s.in_func else s.econsts + s.lconsts)
        for o in objs:
            for e, t in s.descend(o, write):
                if pred(t):
                    c.append((e, t))
        return c

    # ------------------------------------------------------------ expressions
    def vec(s, k, w):
        """An expression of kind k (slv, uns, sgn) and width w."""
        rng, d = s.rng, s.depth
        t = T(k, w=w)
        ops = ["name"] * 5 + ["lit"]
        if d > 0:
            ops += ["bin", "bin", "not", "conv", "concat", "resize", "call", "ctab"]
            if k != "slv":
                ops += ["arith", "arith", "shift", "mul", "toint", "div"]
            else:
                ops += ["arith"]
        op = rng.choice(ops)
        s.depth -= 1
        try:
            return s.vec_op(op, k, w, t)
        finally:
            s.depth += 1

    def vec_op(s, op, k, w, t):
        rng = s.rng
        if op == "name":
            c = s.names(lambda u: u.k in VEC and u.w >= w and (u.k == k or s.chance(0.3)), s.read)
            if not c:
                return s.lit(t)
            e, u = rng.choice(c)
            if u.w > w:
                a = rng.randint(u.lo, u.hi - w + 1)
                e = Ex(f"{{}}({a} to {a + w - 1})" if u.asc else f"{{}}({a + w - 1} downto {a})", T(u.k, w=w), [e])
            return s.conv(e, u.k, k, w)
        if op == "lit":
            return s.lit(t)
        if op == "bin":
            g = rng.choice(["and", "or", "xor", "nand", "nor", "xnor"])
            return Ex("({} %s {})" % g, t, [s.vec(k, w), s.vec(k, w)])
        if op == "not":
            return Ex("(not {})", t, [s.vec(k, w)])
        if op == "conv":
            k2 = rng.choice([x for x in VEC if x != k])
            return s.conv(s.vec(k2, w), k2, k, w)
        if op == "concat" and w >= 2:
            q = {"slv": "std_ulogic_vector", "uns": "unsigned", "sgn": "signed"}[k]
            r = rng.random()
            if r < 0.3:
                return Ex(q + "'({} & {})", t, [s.sl(), s.vec(k, w - 1)])
            if r < 0.5:
                return Ex(q + "'({} & {})", t, [s.vec(k, w - 1), s.sl()])
            a = rng.randint(1, w - 1)
            return Ex(q + "'({} & {})", t, [s.vec(k, a), s.vec(k, w - a)])
        if op == "resize" and k != "slv":
            return Ex(f"resize({{}}, {w})", t, [s.vec(k, s.width())])
        if op == "resize":
            k2 = rng.choice(["uns", "sgn"])
            return s.conv(Ex(f"resize({{}}, {w})", T(k2, w=w), [s.vec(k2, s.width())]), k2, k, w)
        if op == "arith" and k == "slv":
            return Ex("std_ulogic_vector({})", t, [s.vec_op("arith", "uns", w, T("uns", w=w))])
        if op == "arith":
            r = rng.random()
            if r < 0.4:
                return Ex("({} %s {})" % rng.choice("+-"), t, [s.vec(k, w), s.vec(k, rng.randint(1, w))])
            if r < 0.6:
                return Ex("({} %s {})" % rng.choice("+-"), t, [s.vec(k, w), s.sl()])
            if r < 0.8:
                return Ex("({} %s %d)" % (rng.choice("+-"), rng.randint(0, min(300, (1 << (w - 1)) - 1))), t, [s.vec(k, w)])
            if k == "sgn":
                return Ex(rng.choice(["(- {})", "(abs {})"]), t, [s.vec(k, w)])
            return Ex("({} - {})", t, [s.vec(k, rng.randint(1, w)), s.vec(k, w)])
        if op == "shift":
            f = rng.choice(["shift_left", "shift_right", "rotate_left", "rotate_right"] * 10 + ["sll", "srl", "rol", "ror"])
            if w <= 64 and s.chance(0.4):
                n = Ex("to_integer({})", T("int", lo=0, hi=7), [s.vec("uns", 3)])
            else:
                n = Ex(str(rng.randint(0, w + 2)), T("int", lo=0, hi=w + 2))
            if f in ("sll", "srl", "rol", "ror"):
                return Ex("({} %s {})" % f, t, [s.vec(k, w), n])
            return Ex(f"{f}({{}}, {{}})", t, [s.vec(k, w), n])
        if op == "mul":
            a, b = s.width(70), s.width(70)
            return Ex(f"resize({{}} * {{}}, {w})", t, [s.vec(k, a), s.vec(k, b)])
        if op == "toint":
            if k == "uns":
                m = 1 << min(w, 30)
                return Ex(f"to_unsigned({{}}, {w})", t, [s.int_in(0, m - 1)])
            m = 1 << min(w, 30)
            return Ex(f"to_signed({{}}, {w})", t, [s.int_in(-m // 2, m // 2 - 1)])
        if op == "div" and w <= 64:
            a = s.width(64)
            one = f"to_unsigned(1, {w})" if k == "uns" else f"to_signed({1 if w > 1 else -1}, {w})"
            g = rng.choice(["rem", "mod"]) if k == "uns" else "rem"
            if rng.random() < 0.5:  # "/" has the left operand's length
                return Ex(f"({{}} / ({{}} or {one}))", t, [s.vec(k, w), s.vec(k, w)])
            return Ex(f"({{}} {g} ({{}} or {one}))", t, [s.vec(k, a), s.vec(k, w)])
        if op == "call":
            return s.call(t)
        if op == "ctab":
            return s.ctab(t)
        return s.lit(t)

    def conv(s, e, k1, k2, w):
        if k1 == k2:
            return e
        f = {"slv": "std_ulogic_vector", "uns": "unsigned", "sgn": "signed"}[k2]
        return Ex(f + "({})", T(k2, w=w), [e])

    def sl(s):
        rng, d = s.rng, s.depth
        t = T("sl")
        ops = ["name"] * 4 + ["bit"] * 3 + ["lit"]
        if d > 0:
            ops += ["gate", "gate", "not", "b2l", "b2l", "red", "match"]
        op = rng.choice(ops)
        s.depth -= 1
        try:
            if op == "name":
                c = s.names(lambda u: u.k == "sl", s.read)
                if c:
                    return rng.choice(c)[0]
                op = "bit"
            if op == "bit":
                c = s.names(lambda u: u.k in VEC, s.read)
                if c:
                    e, u = rng.choice(c)
                    return Ex("{}({})", t, [e, s.index(u)])
                op = "lit"
            if op == "lit":
                return s.lit(t)
            if op == "gate":
                return Ex("({} %s {})" % rng.choice(["and", "or", "xor", "nand", "nor", "xnor"]), t, [s.sl(), s.sl()])
            if op == "not":
                return Ex("(not {})", t, [s.sl()])
            if op == "b2l":
                return Ex("b2l({})", t, [s.cond()])
            if op == "red":
                w = s.width()
                return Ex(rng.choice(["(and {})", "(or {})", "(xor {})"]), t, [s.vec("slv", w)])
            w = s.width()
            k = rng.choice(["uns", "sgn"])
            return Ex(rng.choice(["({} ?= {})", "({} ?/= {})"]), t, [s.vec(k, w), s.vec(k, w)])
        finally:
            s.depth += 1

    def cond(s):
        rng = s.rng
        r = rng.random()
        s.depth -= 1
        try:
            if r < 0.25:
                return Ex("({} = '1')" if s.chance(0.7) else "({} = '0')", T("bool"), [s.sl()])
            if r < 0.45:
                k = rng.choice(["uns", "sgn"])
                op = rng.choice(["<", "<=", ">", ">=", "=", "/="])
                if s.chance(0.3):
                    v = rng.randint(0, 200) if k == "uns" else rng.randint(-100, 100)
                    return Ex(f"({{}} {op} {v})", T("bool"), [s.vec(k, s.width())])
                return Ex(f"({{}} {op} {{}})", T("bool"), [s.vec(k, s.width()), s.vec(k, s.width())])
            if r < 0.55:
                w = s.width()
                return Ex(f"({{}} {rng.choice(['=', '/='])} {{}})", T("bool"), [s.vec("slv", w), s.vec("slv", w)])
            if r < 0.68:
                op = rng.choice(["<", "<=", ">", ">=", "=", "/="])
                return Ex(f"({{}} {op} {{}})", T("bool"), [s.int_raw(), s.int_raw()])
            if r < 0.78:
                e = rng.choice(s.enums)
                return Ex(f"({{}} {rng.choice(['=', '/='])} {{}})", T("bool"), [s.expr(e), s.expr(e)])
            if r < 0.85 and s.depth > 0:
                return Ex("(not {})", T("bool"), [s.cond()])
            if r < 0.95 and s.depth > 0:
                return Ex("({} %s {})" % rng.choice(["and", "or"]), T("bool"), [s.cond(), s.cond()])
            gb = [o for o in s.read if o.cls == "gen" and o.t.k == "bool"]
            if gb:
                return Ex(rng.choice(gb).name, T("bool"))
            return Ex("({} = '1')", T("bool"), [s.sl()])
        finally:
            s.depth += 1

    def int_raw(s):
        """An integer expression with a known range r (|r| < 2**30)."""
        rng = s.rng
        r = rng.random()
        s.depth -= 1
        try:
            if r < 0.35 or s.depth < 0:
                c = s.names(lambda u: u.k == "int", s.read)
                c = [x for x in c if x[0].r]
                if c and r < 0.3:
                    return rng.choice(c)[0]
                if s.depth >= 0 and s.chance(0.6):
                    w = rng.randint(1, 12)
                    if s.chance(0.7):
                        return Ex("to_integer({})", T("int"), [s.vec("uns", w)], r=(0, (1 << w) - 1))
                    w = max(w, 2)  # numeric_std's TO_INTEGER of a 1-element negative value warns (it adds 1)
                    return Ex("to_integer({})", T("int"), [s.vec("sgn", w)], r=(-(1 << (w - 1)), (1 << (w - 1)) - 1))
                v = rng.randint(-50, 300)
                return Ex(str(v) if v >= 0 else f"({v})", T("int"), r=(v, v))
            if r < 0.45:
                c = s.names(lambda u: u.k in VEC or u.k == "arr", s.read)
                if c:
                    e, u = rng.choice(c)
                    a = rng.choice(["length", "high", "low", "left", "right"])
                    v = {"length": u.n(), "high": u.hi, "low": u.lo, "left": u.lo if u.asc else u.hi, "right": u.hi if u.asc else u.lo}[a]
                    return Ex("{}'" + a, T("int"), [e], r=(v, v))
            if r < 0.5:
                e = rng.choice(s.enums)
                return Ex(f"{e.name}'pos({{}})", T("int"), [s.expr(e)], r=(0, e.n() - 1))
            a, b = s.int_raw(), s.int_raw()
            op = rng.choice(["+", "-", "*", "mod", "rem", "/", "min", "max", "abs"])
            if op in ("mod", "rem", "/"):
                k = rng.randint(1, 37)
                lo, hi = a.r
                rr = {"mod": (0, k - 1), "rem": (min(0, -(k - 1)) if lo < 0 else 0, k - 1 if hi > 0 else 0),
                      "/": (min(lo // k, -(-lo // k)), max(hi // k, -(-hi // k)))}[op]
                rr = (min(rr[0], 0) if op == "rem" else rr[0], rr[1])
                if op == "/":
                    rr = (-(abs(lo) // k) if lo < 0 else lo // k, hi // k if hi >= 0 else -(abs(hi) // k))
                return Ex(f"({{}} {op} {k})", T("int"), [a], r=rr)
            if op == "abs":
                lo, hi = a.r
                return Ex("(abs {})", T("int"), [a], r=(0 if lo <= 0 <= hi else min(abs(lo), abs(hi)), max(abs(lo), abs(hi))))
            if op in ("min", "max"):
                f = {"min": min, "max": max}[op]
                return Ex(f"{op}imum({{}}, {{}})", T("int"), [a, b], r=(f(a.r[0], b.r[0]), f(a.r[1], b.r[1])))
            if op == "*":
                ps = [x * y for x in a.r for y in b.r]
                rr = (min(ps), max(ps))
            elif op == "+":
                rr = (a.r[0] + b.r[0], a.r[1] + b.r[1])
            else:
                rr = (a.r[0] - b.r[1], a.r[1] - b.r[0])
            if max(abs(rr[0]), abs(rr[1])) >= BIG:
                return a
            return Ex(f"({{}} {op} {{}})", T("int"), [a, b], r=rr)
        finally:
            s.depth += 1

    def int_in(s, lo, hi):
        e = s.int_raw()
        if lo <= e.r[0] and e.r[1] <= hi:
            return e
        n = hi - lo + 1
        x = Ex(f"(({{}}) mod {n})" if lo == 0 else f"({lo} + (({{}}) mod {n}))", T("int"), [e], r=(lo, hi))
        return x

    def expr(s, t):
        if t.k == "sl":
            return s.sl()
        if t.k in VEC:
            return s.vec(t.k, t.w)
        if t.k == "int":
            return s.int_in(t.lo, t.hi)
        if t.k == "bool":
            return s.cond()
        c = [x for x in s.names(lambda u: u.k == t.k and u.name == t.name, s.read)]
        rng = s.rng
        if t.k == "enum":
            r = rng.random()
            if c and r < 0.5:
                return rng.choice(c)[0]
            if r < 0.75 or s.depth <= 0:
                return s.lit(t)
            s.depth -= 1
            try:
                return Ex(f"{t.name}'val({{}})", t, [s.int_in(0, t.n() - 1)])
            finally:
                s.depth += 1
        if c and (rng.random() < 0.6 or s.depth <= 0):
            return rng.choice(c)[0]
        if s.depth <= 0:
            return Ex(zero(t), t)
        s.depth -= 1
        try:
            if t.k == "rec":
                fs = [s.expr(ft) for _, ft in t.fields]
                for x, (_, ft) in zip(fs, t.fields):  # a field's aggregate has its context
                    if ft.k == "rec" and x.f.startswith(ft.name + "'(") and s.chance(0.9):
                        x.f = x.f[len(ft.name) + 1:]
                return Ex(f"{t.name}'(" + ", ".join(f"{f} => {{}}" for f, _ in t.fields) + ")", t, fs)
            if s.chance(0.5) or t.n() > 8 or t.n() == 1:
                return Ex(f"{t.name}'(others => {{}})", t, [s.expr(t.elem)])
            return Ex(f"{t.name}'(" + ", ".join("{}" for _ in range(t.n())) + ")", t, [s.expr(t.elem) for _ in range(t.n())])
        finally:
            s.depth += 1

    # ------------------------------------------------------------ constants and subprograms
    def ctab(s, t):
        """An element of a constant table, or a constant, of type t (a vector)."""
        rng = s.rng
        pool = s.pconsts + ([] if s.in_func else s.econsts)
        c = [o for o in pool if o.t.k == "arr" and o.t.elem.k in VEC and o.t.elem.w == t.w] + \
            [o for o in pool if o.t.k in VEC and o.t.w == t.w]
        if not c or s.chance(0.3):
            if rng.random() < 0.6:
                n = rng.choice([1, 2, 4, 8, 16, 3])
                lo = rng.choice([0, 0, 1, 2])
                et = s.vtype(t.k, t.w)
                at = T("arr", name=s.fresh("rom") + "_t", elem=et, lo=lo, hi=lo + n - 1, asc=s.chance(0.7))
                o = Obj(s.fresh("ROM"), at, "const")
                s.arrs_local.append(at)
            else:
                o = Obj(s.fresh("K"), s.vtype(t.k, t.w), "const")
            (s.pconsts if s.in_func or s.chance(0.5) else s.econsts).append(o)
            if o in s.pconsts and o.t.k == "arr":
                s.arrs_local.remove(o.t)
                s.arrs.append(o.t)
            c = [o]
        o = rng.choice(c)
        e = Ex(o.name, o.t)
        if o.t.k == "arr":
            e = Ex("{}({})", o.t.elem, [e, s.index(o.t)])
            u = o.t.elem
        else:
            u = o.t
        return s.conv(e, u.k, t.k, t.w)

    def call(s, t):
        """A call of a function of the package returning t."""
        rng = s.rng
        c = [f for f in s.funcs if f["ret"].k == t.k and f["ret"].w == t.w]
        if not c or (len(s.funcs) < 8 and s.chance(0.4)):
            c = [s.make_func(t)]
        f = rng.choice(c)
        return Ex(f["name"] + "(" + ", ".join("{}" for _ in f["params"]) + ")", t, [s.expr(p.t) for p in f["params"]])

    def make_func(s, ret):
        rng = s.rng
        name = s.fresh("fn")
        params = [Obj(f"p{i}", s.scalar_type() if i else s.vtype(ret.k, s.width()), "var") for i in range(rng.randint(1, 3))]
        locs = [Obj(f"l{i}", s.scalar_type(), "var") for i in range(rng.randint(0, 3))]
        rt = T(ret.k, w=ret.w)
        f = {"name": name, "params": params, "ret": rt, "locs": locs}
        saved = (s.read, s.write, s.depth, s.loopv, s.in_func)
        s.read, s.write, s.depth, s.loopv, s.in_func = params + locs + s.pconsts, locs, 2, [], True
        try:
            f["body"] = s.stmts(rng.randint(1, 4), 2, var_only=True)
            f["ret_e"] = s.expr(rt)
        finally:
            s.read, s.write, s.depth, s.loopv, s.in_func = saved
        s.funcs.append(f)
        return f

    def make_proc(s):
        rng = s.rng
        name = s.fresh("pr")
        params = [Obj(f"a{i}", s.scalar_type(), "var") for i in range(rng.randint(1, 2))]
        outs = [Obj(f"y{i}", s.scalar_type(), "var") for i in range(rng.randint(1, 2))]
        saved = (s.read, s.write, s.depth, s.loopv, s.in_func)
        s.read, s.write, s.depth, s.loopv, s.in_func = params + outs + s.pconsts, outs, 2, [], True
        try:
            body = [St("assign", Ex(o.name, o.t), ":=", s.expr(o.t)) for o in outs]
            body += s.stmts(rng.randint(0, 3), 2, var_only=True)
        finally:
            s.read, s.write, s.depth, s.loopv, s.in_func = saved
        p = {"name": name, "params": params, "outs": outs, "body": body}
        s.procs.append(p)
        return p

    # ------------------------------------------------------------ statements
    def target(s, var_only=False):
        """(target Ex, its type, op) of something the current process may write."""
        objs = [o for o in s.write if not var_only or o.cls == "var"]
        if not objs:
            return None
        o = s.rng.choice(objs)
        path = s.descend(o, write=True)
        e, t = s.rng.choice(path)
        if t.k in VEC and t.w > 1 and s.chance(0.35):
            r = s.rng.random()
            if r < 0.4:
                e, t = Ex("{}({})", T("sl"), [e, s.index(t, True)]), T("sl")
            else:
                w = s.rng.randint(1, t.w)
                a = s.rng.randint(t.lo, t.hi - w + 1)
                e = Ex(f"{{}}({a} to {a + w - 1})" if t.asc else f"{{}}({a + w - 1} downto {a})", T(t.k, w=w), [e])
                t = T(t.k, w=w)
        return e, t, ":=" if o.cls == "var" else "<="

    def stmts(s, n, d, var_only=False):
        return [x for x in (s.stmt(d, var_only) for _ in range(n)) if x]

    def stmt(s, d, var_only=False):
        rng = s.rng
        r = rng.random()
        s.depth = rng.randint(1, 3)
        if d > 0 and r < 0.15:
            arms = [(s.cond(), s.stmts(rng.randint(1, 3), d - 1, var_only))]
            while s.chance(0.3):
                arms.append((s.cond(), s.stmts(rng.randint(1, 2), d - 1, var_only)))
            return St("if", arms, s.stmts(rng.randint(1, 2), d - 1, var_only) if s.chance(0.6) else None)
        if d > 0 and r < 0.23:
            return s.case_stmt(d, var_only)
        if d > 0 and r < 0.3:
            return s.for_stmt(d, var_only)
        if r < 0.34 and s.loopv:
            return St(rng.choice(["exit", "next"]), s.cond())
        if r < 0.37 and var_only is False and any(o.cls == "var" for o in s.write):
            p = rng.choice(s.procs) if s.procs and s.chance(0.6) else s.make_proc()
            outs = []
            for y in p["outs"]:
                c = [o for o in s.write if o.cls == "var" and o.t.k == y.t.k and o.t.same(y.t) and y.t.same(o.t)]
                if not c:
                    return None
                outs.append(Ex(rng.choice(c).name, y.t))
            args = [s.expr(a.t) for a in p["params"]]
            return St("call", p["name"], args + outs)
        tg = s.target(var_only)
        if not tg:
            return None
        e, t, op = tg
        if r < 0.45 and t.k != "arr" and t.k != "rec":
            arms = [(s.expr(t), s.cond()) for _ in range(rng.randint(1, 2))]
            return St("cassign", e, op, arms, s.expr(t))
        if t.k in VEC and not e.a and t.w > 1 and s.chance(0.15):  # an aggregate with named or range choices
            a = rng.randint(t.lo, t.hi)
            b = rng.randint(a, t.hi)
            rng_txt = f"{a} to {b}" if t.asc else f"{b} downto {a}"
            if s.chance(0.5) and b > a:
                return St("assign", e, op, Ex(f"({rng_txt} => {{}}, others => {{}})", t, [s.vec(t.k, b - a + 1), s.sl()]))
            return St("assign", e, op, Ex(f"({a} => {{}}, others => {{}})", t, [s.sl(), s.sl()]))
        rhs = s.expr(t)
        if t.k == "rec" and rhs.f.startswith(t.name + "'(") and s.chance(0.9):  # an aggregate in context
            rhs = Ex(rhs.f[len(t.name) + 1:], t, rhs.a)
        return St("assign", e, op, rhs)

    def case_stmt(s, d, var_only):
        rng = s.rng
        if s.chance(0.6):
            et = rng.choice(s.enums)
            sel = s.expr(et)
            lits = et.lits[:]
            rng.shuffle(lits)
            arms, i = [], 0
            while i < len(lits):
                k = rng.randint(1, 2)
                arms.append((" | ".join(lits[i:i + k]), s.stmts(rng.randint(0, 2), d - 1, var_only)))
                i += k
            if s.chance(0.3) and len(arms) > 1:
                arms[-1] = ("others", arms[-1][1])
            return St("case", sel, arms)
        w = rng.randint(1, 4)
        c = s.names(lambda u: u.k in VEC and u.w == w, s.read)
        if c:
            sel = rng.choice(c)[0]
        else:
            sel = Ex("unsigned'({})", T("uns", w=w), [s.vec("uns", w)])
        vals = rng.sample(range(1 << w), min(1 << w, rng.randint(1, 4)))
        arms = [(f'"{v:0{w}b}"', s.stmts(rng.randint(0, 2), d - 1, var_only)) for v in vals]
        arms.append(("others", s.stmts(rng.randint(0, 1), d - 1, var_only)))
        return St("case", sel, arms)

    def for_stmt(s, d, var_only):
        rng = s.rng
        v = s.fresh("i")
        c = s.names(lambda u: u.k in VEC and u.w <= 140, s.read)
        if c and s.chance(0.6):
            e, u = rng.choice(c)
            rng_t, lo, hi = rng.choice([("{}'range", u.lo, u.hi), ("{}'reverse_range", u.lo, u.hi)]), u.lo, u.hi
            rtext = Ex(rng_t[0], T("int"), [e])
        else:
            lo = rng.randint(0, 3)
            hi = lo + rng.randint(0, 9)
            rtext = Ex(rng.choice([f"{lo} to {hi}", f"{hi} downto {lo}"]), T("int"))
        o = Obj(v, T("int", lo=lo, hi=hi), "loop")
        s.read.append(o)
        s.loopv.append(o)
        try:
            body = []
            # element-wise work through the loop index: v(i + c) on vectors that hold it
            for _ in range(rng.randint(1, 3)):
                ts = [x for x in s.write if x.t.k in VEC and x.t.w >= hi - lo + 1 and (not var_only or x.cls == "var")]
                if ts and s.chance(0.6):
                    x = rng.choice(ts)
                    off = x.t.lo - lo
                    ix = v if off == 0 else f"{v} + {off}" if off > 0 else f"{v} - {-off}"
                    s.depth = 2
                    rhs = s.sl()
                    srcs = [y for y in s.read if y.t.k in VEC and y.t.w >= hi - lo + 1 and y.cls != "loop"]
                    if srcs and s.chance(0.6):
                        y = rng.choice(srcs)
                        o2 = y.t.lo - lo
                        iy = v if o2 == 0 else f"{v} + {o2}" if o2 > 0 else f"{v} - {-o2}"
                        rhs = Ex("({} %s {})" % rng.choice(["xor", "and", "or"]), T("sl"),
                                 [Ex(f"{y.name}({iy})", T("sl"), sig=y.name if y.cls == "sig" else None), rhs])
                    body.append(St("assign", Ex(f"{x.name}({ix})", T("sl"), sig=None), ":=" if x.cls == "var" else "<=", rhs))
                else:
                    body += s.stmts(1, d - 1, var_only)
        finally:
            s.read.remove(o)
            s.loopv.remove(o)
        return St("for", v, rtext, body)

    # ------------------------------------------------------------ entities
    def entity(s, depth, top=False):
        rng = s.rng
        ent = Ent("fz_top" if top else s.fresh("fz_sub"))
        s.ents.append(ent)
        s.econsts, s.arrs_local, s.lconsts = ent.consts, [], []
        for _ in range(rng.randint(0, 3)):  # constants of any type, for their parts
            (s.pconsts if s.chance(0.4) else s.econsts).append(Obj(s.fresh("K"), s.any_type(), "const"))
        ent.generics = [Obj("G_K", T("int", lo=0, hi=15), "gen"), Obj("G_B", T("bool"), "gen")]
        ent.generics[0].t.r = (0, 15)
        clocks = ["clk0", "clk1"]
        ins = [Obj(c, T("sl"), "sig") for c in clocks] + [Obj("rst", T("sl"), "sig"), Obj("arst", T("sl"), "sig")]
        n_in = max(1, int(rng.randint(2, 6) * s.scale))
        ins += [Obj(s.fresh("i"), s.any_type(), "sig") for _ in range(n_in)]
        ent.ports = [("in", o) for o in ins]
        data_in = ins[4:]
        regs = [Obj(s.fresh("q"), s.any_type(), "sig") for _ in range(max(1, int(rng.randint(2, 6) * s.scale)))]
        mems = []
        if s.arrs and s.chance(0.5):
            mems = [Obj(s.fresh("m"), rng.choice(s.arrs), "sig")]
        avail = data_in + regs + mems + list(ent.generics)
        sigs = regs + mems
        # combinational logic in order: each reads inputs, registers and earlier logic
        for _ in range(max(1, int(rng.randint(2, 7) * s.scale))):
            r = rng.random()
            if r < 0.15 and depth > 0:
                outs = s.instance(ent, depth, avail)
                avail += outs
                sigs += outs
                continue
            outs = [Obj(s.fresh("c"), s.any_type(), "sig") for _ in range(rng.randint(1, 2))]
            s.read = [o for o in avail]
            if r < 0.12:
                outs = s.out_proc(ent, outs[:1])
            elif r < 0.5:
                s.comb_proc(ent, outs)
            else:
                outs = outs[:1]
                s.conc(ent, outs[0])
            avail += outs
            sigs += outs
        # registers: their processes read everything
        groups = {}
        for q in regs + mems:
            groups.setdefault(rng.randint(0, max(0, len(regs) // 2)), []).append(q)
        for g in groups.values():
            s.read = [o for o in avail]
            if len(g) == 1 and g[0] not in mems and s.chance(0.15):  # q <= d when rising_edge(clk);
                s.depth = rng.randint(1, 3)
                ent.procs.append(Proc("whenclk", tgt=g[0], e=s.expr(g[0].t), drives=g,
                                      edge=rng.choice(["rising_edge", "falling_edge"]) + "(" + ("clk1" if s.chance(0.15) else "clk0") + ")"))
                continue
            s.clocked(ent, g, mems)
        ent.sigs = sigs
        names = [o.name for o in sigs]
        ent.out_names = set(names) if top else set(n for n in names if rng.random() < 0.6) or set(names[:1])
        ent.ports += [("out", o) for o in sigs if o.name in ent.out_names]
        return ent

    def instance(s, ent, depth, avail):
        rng = s.rng
        saved = (s.econsts, s.arrs_local, s.read, s.lconsts)
        sub = s.entity(depth - 1)
        s.econsts, s.arrs_local, s.read, s.lconsts = saved
        outs_all = []
        for k in range(2 if s.chance(0.3) else 1):
            s.read = list(avail)
            gk = rng.randint(0, 15)
            maps = []
            for mode, o in sub.ports:
                if o.name in ("clk0", "clk1", "rst", "arst"):
                    maps.append((o.name, Ex(rng.choice(["clk0", "clk1"]) if o.name.startswith("clk") and s.chance(0.1) else o.name, o.t, sig=o.name)))
                    continue
                if mode == "in":
                    s.depth = rng.randint(0, 2)
                    if o.t.k in VEC and o.t.w >= 2 and s.chance(0.25):
                        a = rng.randint(1, o.t.w - 1)
                        tl, th = T(o.t.k, w=a), T(o.t.k, w=o.t.w - a)
                        lo = o.t.lo
                        p1 = f"{o.name}({lo + a - 1} downto {lo})" if not o.t.asc else f"{o.name}({lo} to {lo + a - 1})"
                        p2 = f"{o.name}({o.t.hi} downto {lo + a})" if not o.t.asc else f"{o.name}({lo + a} to {o.t.hi})"
                        maps.append((p1, s.vec(o.t.k, a)))
                        maps.append((p2, s.vec(o.t.k, o.t.w - a)))
                    else:
                        c = s.names(lambda u: u.k == o.t.k and u.name == o.t.name, s.read) if o.t.k in ("rec", "arr") else []
                        maps.append((o.name, rng.choice(c)[0] if c and s.chance(0.8) else s.expr(o.t)))
                else:
                    if s.chance(0.08):
                        maps.append((o.name, None))
                        continue
                    if o.t.k in VEC and o.t.w >= 2 and s.chance(0.2):
                        a = rng.randint(1, o.t.w - 1)
                        lo = o.t.lo
                        x1 = Obj(s.fresh("c"), T(o.t.k, w=a), "sig")
                        x2 = Obj(s.fresh("c"), T(o.t.k, w=o.t.w - a), "sig")
                        p1 = f"{o.name}({lo + a - 1} downto {lo})" if not o.t.asc else f"{o.name}({lo} to {lo + a - 1})"
                        p2 = f"{o.name}({o.t.hi} downto {lo + a})" if not o.t.asc else f"{o.name}({lo + a} to {o.t.hi})"
                        maps.append((p1, Ex(x1.name, x1.t, sig=x1.name)))
                        maps.append((p2, Ex(x2.name, x2.t, sig=x2.name)))
                        outs_all += [x1, x2]
                    else:
                        x = Obj(s.fresh("c"), o.t, "sig")
                        maps.append((o.name, Ex(x.name, x.t, sig=x.name)))
                        outs_all.append(x)
            p = Proc("inst", sub=sub.name, gk=gk, gb=rng.choice(["true", "false"]), maps=maps)
            p.label = s.fresh("u")
            ent.procs.append(p)
        return outs_all

    def local_consts(s):
        """0 or 1 constant declared in the process, readable by its statements."""
        cs = []
        if s.chance(0.3):
            c = Obj(s.fresh("LK"), s.any_type(), "const")
            c.v = s.const_text(c.t)
            cs.append(c)
        s.lconsts = cs
        return cs

    def comb_proc(s, ent, outs):
        rng = s.rng
        lc = s.local_consts()
        vs = [Obj(s.fresh("v"), s.any_type(), "var") for _ in range(rng.randint(0, 2))]
        s.write = outs + vs
        s.loopv = []
        s.depth = 2
        body = [St("assign", Ex(o.name, o.t), ":=" if o.cls == "var" else "<=", s.expr(o.t)) for o in vs + outs]
        s.read = s.read + vs  # only after the default assignments: no variable read before it is assigned
        body += s.stmts(rng.randint(1, 5), 2)
        if outs[0].t.k == "rec" and s.chance(0.5):  # the two-process record style
            v = Obj(s.fresh("v"), outs[0].t, "var")
            vs.append(v)
            body = [St("assign", Ex(v.name, v.t), ":=", s.expr(v.t))] + body + [St("assign", Ex(outs[0].name, v.t), "<=", Ex(v.name, v.t))]
        p = Proc("comb", decls=lc + vs, body=body, drives=outs, all=s.chance(0.6))
        ent.procs.append(p)
        s.lconsts = []

    def out_proc(s, ent, outs):
        """A process cycle mode must leave in the event kernel, with the normal result: a latch, a
        variable keeping state, an incomplete sensitivity list, or waits in time."""
        rng = s.rng
        s.write, s.loopv, s.depth = outs, [], 2
        o = outs[0]
        k = rng.choice(["latch", "state", "sens", "sens", "wait"])
        if k == "latch":
            body = [St("if", [(s.cond(), [St("assign", Ex(o.name, o.t), "<=", s.expr(o.t))])], None)]
            ent.procs.append(Proc("comb", decls=[], body=body, drives=outs, all=True))
        elif k == "state":
            v = Obj(s.fresh("v"), o.t, "var")
            s.read = s.read + [v]
            body = [St("assign", Ex(o.name, o.t), "<=", s.expr(o.t)), St("assign", Ex(v.name, v.t), ":=", s.expr(v.t))]
            ent.procs.append(Proc("comb", decls=[v], body=body, drives=outs, all=True))
        elif k == "sens":  # reads logic that follows a register, waits on the register alone
            qs = [x for x in s.read if x.cls == "sig" and x.name.startswith("q") and x.t.k in VEC]
            if qs:
                q = rng.choice(qs)
                c = Obj(s.fresh("c"), T("uns", w=8), "sig")
                ent.procs.append(Proc("conc", tgt=c, e=Ex(f"resize(unsigned({q.name}), 8) + {rng.randint(1, 9)}", c.t)))
                outs = outs + [c]
                o.t = T("uns", w=8)
                e = Ex(f"resize(unsigned({c.name}), 8) + resize(unsigned({q.name}), 8)", o.t)
                ent.procs.append(Proc("comb", decls=[], body=[St("assign", Ex(o.name, o.t), "<=", e)], drives=outs, all=False, sens=[q.name]))
            else:
                body = [St("assign", Ex(o.name, o.t), "<=", s.expr(o.t))]
                ent.procs.append(Proc("comb", decls=[], body=body, drives=outs, all=False, partial=True))
        else:
            body = [St("assign", Ex(o.name, o.t), "<=", s.expr(o.t))]
            ent.procs.append(Proc("waits", decls=[], body=body, drives=outs, clk="clk1" if s.chance(0.2) else "clk0"))
        return outs

    def conc(s, ent, out):
        rng = s.rng
        s.depth = rng.randint(1, 3)
        r = rng.random()
        if r < 0.5 or out.t.k in ("rec", "arr"):
            ent.procs.append(Proc("conc", tgt=out, e=s.expr(out.t)))
        elif r < 0.75:
            arms = [(s.expr(out.t), s.cond()) for _ in range(rng.randint(1, 3))]
            ent.procs.append(Proc("cond", tgt=out, arms=arms, els=s.expr(out.t)))
        elif r < 0.9 or out.t.k not in VEC:
            et = rng.choice(s.enums)
            sel = s.expr(et)
            lits = et.lits[:]
            rng.shuffle(lits)
            k = rng.randint(1, len(lits))
            arms = [(s.expr(out.t), lit) for lit in lits[:k]]
            ent.procs.append(Proc("sel", tgt=out, sel=sel, arms=arms, els=s.expr(out.t) if k < len(lits) else None))
        else:  # a for generate driving each element
            s.read.append(Obj("gi", T("int", lo=out.t.lo, hi=out.t.hi), "loop"))
            srcs = [y for y in s.read if y.t.k in VEC and y.t.w >= out.t.w and y.cls != "loop"]
            e = s.sl()
            if srcs:
                y = rng.choice(srcs)
                off = y.t.lo - out.t.lo
                iy = "gi" if off == 0 else f"gi + {off}" if off > 0 else f"gi - {-off}"
                e = Ex("({} %s {})" % rng.choice(["xor", "and", "or"]), T("sl"), [Ex(f"{y.name}({iy})", T("sl"), sig=y.name), e])
            s.read.pop()
            ent.procs.append(Proc("gen_bits", tgt=out, e=e))

    def clocked(s, ent, regs, mems):
        rng = s.rng
        clk = "clk1" if s.chance(0.12) else "clk0"
        edge = rng.choice(["rising_edge({c})", "rising_edge({c})", "falling_edge({c})", "{c}'event and {c} = '1'", "{c} = '1' and {c}'event"]).format(c=clk)
        reset = rng.choice(["sync", "sync", "async", "none"])
        lc = s.local_consts()
        vs = [Obj(s.fresh("v"), s.any_type(), "var") for _ in range(rng.randint(0, 2))]
        s.write = regs + vs
        s.read = s.read + vs
        s.loopv = []
        body = []
        for m in [x for x in regs if x in mems]:  # a write port at a run-time index
            s.depth = 2
            we = s.cond()
            ix = s.index(m.t)
            if ix.a == []:
                s.depth = 1
                ix = s.index(m.t)
            body.append(St("if", [(we, [St("assign", Ex("{}({})", m.t.elem, [Ex(m.name, m.t, sig=m.name), ix]), "<=", s.expr(m.t.elem))])], None))
        body += s.stmts(rng.randint(1, 6) + len(regs), 3)
        for q in regs:  # assign every register somewhere too
            if q not in mems and s.chance(0.5):
                s.depth = 2
                body.append(St("assign", Ex(q.name, q.t, sig=q.name), "<=", s.expr(q.t)))
        rst_body = [St("assign", Ex(q.name, q.t), "<=", Ex(s.const_text(q.t) if q.t.k not in ("int", "enum", "sl") else s.lit(q.t).text(), q.t))
                    for q in regs if q not in mems]
        rst_body += [St("assign", Ex(v.name, v.t), ":=", Ex(s.const_text(v.t) if v.t.k not in ("int", "enum", "sl") else s.lit(v.t).text(), v.t)) for v in vs if s.chance(0.5)]
        en = None
        if s.chance(0.3):
            s.depth = 1
            en = s.cond()
        shape = "proc"
        if reset != "async" and s.chance(0.2):
            shape = "wait"
        s.lconsts = []
        rsig = "arst"
        if reset == "async" and s.chance(0.25):  # a reset made by a register of the design (another process's)
            c = [o for o in s.read if o.cls == "sig" and o.t.k == "sl" and o.name.startswith("q") and o not in regs]
            rsig = rng.choice(c).name if c else rsig
        p = Proc("clocked", decls=lc + vs, body=body, rst_body=rst_body, drives=regs, clk=clk, edge=edge, reset=reset, en=en, shape=shape,
                 rsig=rsig)
        ent.procs.append(p)

    # ------------------------------------------------------------ the design
    def design(s):
        s.make_types()
        s.econsts, s.arrs_local, s.read, s.write, s.loopv, s.depth, s.lconsts = [], [], [], [], [], 2, []
        s.in_func = False
        top = s.entity(depth=s.rng.choice([0, 1, 1, 2]), top=True)
        return top


# ---------------------------------------------------------------- rendering
def ind(n):
    return "  " * n


def render_stmts(sts, n):
    out = []
    for st in sts:
        k, a = st.k, st.a
        if k == "assign":
            out.append(f"{ind(n)}{a[0].text()} {a[1]} {a[2].text()};")
        elif k == "cassign":
            arms = " else ".join(f"{e.text()} when {c.text()}" for e, c in a[2])
            out.append(f"{ind(n)}{a[0].text()} {a[1]} {arms} else {a[3].text()};")
        elif k == "if":
            for i, (c, body) in enumerate(a[0]):
                out.append(f"{ind(n)}{'if' if i == 0 else 'elsif'} {c.text()} then")
                out += render_stmts(body, n + 1) or [ind(n + 1) + "null;"]
            if a[1] is not None:
                out.append(f"{ind(n)}else")
                out += render_stmts(a[1], n + 1) or [ind(n + 1) + "null;"]
            out.append(f"{ind(n)}end if;")
        elif k == "case":
            out.append(f"{ind(n)}case {a[0].text()} is")
            for ch, body in a[1]:
                out.append(f"{ind(n + 1)}when {ch} =>")
                out += render_stmts(body, n + 2) or [ind(n + 2) + "null;"]
            if a[1][-1][0] != "others" and a[0].t.k != "enum":
                out.append(f"{ind(n + 1)}when others => null;")
            out.append(f"{ind(n)}end case;")
        elif k == "for":
            out.append(f"{ind(n)}for {a[0]} in {a[1].text()} loop")
            out += render_stmts(a[2], n + 1) or [ind(n + 1) + "null;"]
            out.append(f"{ind(n)}end loop;")
        elif k in ("exit", "next"):
            out.append(f"{ind(n)}{k} when {a[0].text()};")
        elif k == "call":
            out.append(f"{ind(n)}{a[0]}(" + ", ".join(x.text() for x in a[1]) + ");")
    return out


def walk_stmts(sts):
    for st in sts:
        yield st
        for x in st.a:
            if isinstance(x, list):
                for y in x:
                    if isinstance(y, St):
                        yield from walk_stmts([y])
                    elif isinstance(y, tuple):
                        for z in y:
                            if isinstance(z, list):
                                yield from walk_stmts(z)


def exprs_of(st):
    for x in st.a:
        if isinstance(x, Ex):
            yield x
        elif isinstance(x, list):
            for y in x:
                if isinstance(y, Ex):
                    yield y
                elif isinstance(y, tuple):
                    for z in y:
                        if isinstance(z, Ex):
                            yield z


def proc_exprs(p):
    """Every expression of a process (statements' and its own)."""
    for st in walk_stmts(p.body + p.rst_body):
        yield from exprs_of(st)
    for k in ("e", "els", "sel", "en"):
        if getattr(p, k, None) is not None:
            yield getattr(p, k)
    for a in getattr(p, "arms", None) or []:
        for z in a:
            if isinstance(z, Ex):
                yield z
    for _, e in getattr(p, "maps", None) or []:
        if e is not None:
            yield e


def reads(p):
    out, mine = [], {o.name for o in p.drives}
    for e in proc_exprs(p):
        for x in e.walk():
            if x.sig and x.sig not in out and x.sig not in mine:
                out.append(x.sig)
    return out


def render_proc(p, n=1):
    lab = (p.label + " : ") if p.label else ""
    if p.k == "inst":
        maps = ",\n".join(f"{ind(n + 2)}{f} => {'open' if e is None else e.text()}" for f, e in p.maps)
        return (f"{ind(n)}{p.label} : entity work.{p.sub}\n{ind(n + 1)}generic map (G_K => {p.gk}, G_B => {p.gb})\n"
                f"{ind(n + 1)}port map (\n{maps});")
    if p.k == "conc":
        return f"{ind(n)}{lab}{p.tgt.name} <= {p.e.text()};"
    if p.k == "whenclk":
        return f"{ind(n)}{lab}{p.tgt.name} <= {p.e.text()} when {p.edge};"
    if p.k == "cond":
        arms = " else ".join(f"{e.text()} when {c.text()}" for e, c in p.arms)
        return f"{ind(n)}{lab}{p.tgt.name} <= {arms} else {p.els.text()};"
    if p.k == "sel":
        arms = [f"{e.text()} when {c}" for e, c in p.arms]
        if p.els is not None:
            arms.append(f"{p.els.text()} when others")
        return f"{ind(n)}{lab}with {p.sel.text()} select {p.tgt.name} <=\n{ind(n + 2)}" + (",\n" + ind(n + 2)).join(arms) + ";"
    if p.k == "gen_bits":
        g = p.label or "g_" + p.tgt.name
        return (f"{ind(n)}{g}_gen : for gi in {p.tgt.name}'range generate\n{ind(n + 1)}{p.tgt.name}(gi) <= {p.e.text()};\n"
                f"{ind(n)}end generate;")
    decls = [f"{ind(n + 1)}constant {v.name} : {v.t.mark()} := {v.v};" if v.cls == "const" else
             f"{ind(n + 1)}variable {v.name} : {v.t.mark()} := {zero(v.t)};" for v in p.decls]
    if p.k == "waits":
        return "\n".join([f"{ind(n)}{lab}process"] + decls + [f"{ind(n)}begin", f"{ind(n + 1)}wait until rising_edge({p.clk});",
                          f"{ind(n + 1)}wait for 1 ns;"] + render_stmts(p.body, n + 1) + [f"{ind(n)}end process;"])
    if p.k == "comb":
        rd = getattr(p, "sens", None) or reads(p)
        if getattr(p, "partial", False):
            rd = rd[:1]
        sens = "all" if p.all else ", ".join(rd) or "all"
        return "\n".join([f"{ind(n)}{lab}process ({sens})"] + decls + [f"{ind(n)}begin"] + render_stmts(p.body, n + 1) + [f"{ind(n)}end process;"])
    # clocked
    body = render_stmts(p.body, 0)
    if p.en is not None:
        body = [f"if {p.en.text()} then"] + ["  " + l for l in body] + ["end if;"]
    rst = render_stmts(p.rst_body, 0) or ["null;"]
    if p.reset == "async":
        lines = [f"if {p.rsig} = '1' then"] + ["  " + l for l in rst] + [f"elsif {p.edge} then"] + ["  " + l for l in body] + ["end if;"]
        sens = f"({p.clk}, {p.rsig})"
    else:
        if p.reset == "sync":
            body = ["if rst = '1' then"] + ["  " + l for l in rst] + ["else"] + ["  " + l for l in body] + ["end if;"]
        if p.shape == "wait":
            lines = [f"wait until {p.edge.replace('rising_edge(', 'rising_edge(') };"] + body
            sens = ""
        else:
            lines = [f"if {p.edge} then"] + ["  " + l for l in body] + ["end if;"]
            sens = f"({p.clk})"
    return "\n".join([f"{ind(n)}{lab}process {sens}".rstrip()] + decls + [f"{ind(n)}begin"] + [ind(n + 1) + l for l in lines] + [f"{ind(n)}end process;"])


HEAD = "library ieee;\nuse ieee.std_logic_1164.all;\nuse ieee.numeric_std.all;\nuse work.fz_pkg.all;\n"


def render_pkg(g):
    out = ["library ieee;", "use ieee.std_logic_1164.all;", "use ieee.numeric_std.all;", "package fz_pkg is"]
    for e in g.enums:
        out.append(f"  type {e.name} is ({', '.join(e.lits)});")
    declared = set()

    def decl(t):
        if t.k == "rec" and t.name not in declared:
            for _, ft in t.fields:
                decl(ft)
            declared.add(t.name)
            out.append(f"  type {t.name} is record")
            out.extend(f"    {f} : {ft.mark()};" for f, ft in t.fields)
            out.append("  end record;")
        elif t.k == "arr" and t.name not in declared:
            decl(t.elem)
            declared.add(t.name)
            rng = f"{t.lo} to {t.hi}" if t.asc else f"{t.hi} downto {t.lo}"
            out.append(f"  type {t.name} is array ({rng}) of {t.elem.mark()};")
    for t in g.recs + g.arrs:
        decl(t)
    out.append("  function b2l(b : boolean) return std_ulogic;")
    for c in g.pconsts:
        out.append(f"  constant {c.name} : {c.t.mark()} := {c.v};")
    for f in g.funcs:
        out.append(f"  subtype {f['name']}_t is {f['ret'].mark()};")
        out.append(f"  function {f['name']}(" + "; ".join(f"{p.name} : {p.t.mark()}" for p in f["params"]) + f") return {f['name']}_t;")
    for p in g.procs:
        out.append(f"  procedure {p['name']}(" + "; ".join([f"{a.name} : in {a.t.mark()}" for a in p["params"]] + [f"{y.name} : out {y.t.mark()}" for y in p["outs"]]) + ");")
    out.append("end package;")
    out.append("package body fz_pkg is")
    out.append("  function b2l(b : boolean) return std_ulogic is\n  begin\n    if b then\n      return '1';\n    end if;\n    return '0';\n  end function;")
    for f in g.funcs:
        out.append(f"  function {f['name']}(" + "; ".join(f"{p.name} : {p.t.mark()}" for p in f["params"]) + f") return {f['name']}_t is")
        out += [f"    variable {v.name} : {v.t.mark()} := {zero(v.t)};" for v in f["locs"]]
        out.append("  begin")
        out += render_stmts(f["body"], 2)
        out.append(f"    return {f['ret_e'].text()};")
        out.append("  end function;")
    for p in g.procs:
        out.append(f"  procedure {p['name']}(" + "; ".join([f"{a.name} : in {a.t.mark()}" for a in p["params"]] + [f"{y.name} : out {y.t.mark()}" for y in p["outs"]]) + ") is")
        out.append("  begin")
        out += render_stmts(p["body"], 2)
        out.append("  end procedure;")
    out.append("end package body;")
    return "\n".join(out)


def init_of(g, o):
    return getattr(o, "init", None) or zero(o.t)


def render_ent(g, ent, top):
    ports = []
    for mode, o in ent.ports:
        ports.append(f"    {o.name} : in {o.t.mark()}" if mode == "in" else f"    {o.name} : out {o.t.mark()} := {init_of(g, o)}")
    port_names = {o.name for _, o in ent.ports}
    lines = [HEAD + f"entity {ent.name} is", "  generic (G_K : integer range 0 to 15 := 0; G_B : boolean := false);",
             "  port (\n" + ";\n".join(ports) + ");", "end entity;", f"architecture a of {ent.name} is"]
    for c in ent.consts:
        if c.t.k == "arr" and c.t not in g.arrs:
            rng = f"{c.t.lo} to {c.t.hi}" if c.t.asc else f"{c.t.hi} downto {c.t.lo}"
            lines.append(f"  type {c.t.name} is array ({rng}) of {c.t.elem.mark()};")
        lines.append(f"  constant {c.name} : {c.t.mark()} := {c.v};")
    for o in ent.sigs:
        if o.name not in port_names and o.name not in ent.out_names:
            lines.append(f"  signal {o.name} : {o.t.mark()} := {init_of(g, o)};")
    lines.append("begin")
    for p in ent.procs:
        lines.append(render_proc(p))
    lines.append("end architecture;")
    return "\n".join(lines)


def img(name, t):
    """A VHDL string expression showing the value of `name` of type t."""
    if t.k == "sl":
        return f"std_ulogic'image({name})"
    if t.k in VEC:
        return f"to_hstring({name})"
    if t.k == "int":
        return f"integer'image({name})"
    if t.k == "enum":
        return f"{t.name}'image({name})"
    if t.k == "rec":
        return " & \",\" & ".join(img(f"{name}.{f}", ft) for f, ft in t.fields)
    return " & \",\" & ".join(img(f"{name}({i})", t.elem) for i in range(t.lo, t.hi + 1))


def draw_stmts(tgt, t, n):
    """Statements of the testbench that give `tgt` a random value of type t (procedure draw)."""
    if t.k == "sl":
        return [f"draw(r1); {tgt} <= r1(0);"]
    if t.k in VEC:
        f = {"slv": "", "uns": "unsigned", "sgn": "signed"}[t.k]
        return [f"draw(r{t.w}); {tgt} <= {f}(r{t.w});"]
    if t.k == "int":
        return [f"draw(r16); {tgt} <= {t.lo} + to_integer(unsigned(r16)) mod {t.hi - t.lo + 1};"]
    if t.k == "enum":
        return [f"draw(r8); {tgt} <= {t.name}'val(to_integer(unsigned(r8)) mod {t.n()});"]
    if t.k == "rec":
        return [x for f, ft in t.fields for x in draw_stmts(f"{tgt}.{f}", ft, n)]
    return [x for i in range(t.lo, t.hi + 1) for x in draw_stmts(f"{tgt}({i})", t.elem, n)]


def widths_of(t, acc):
    if t.k in VEC:
        acc.add(t.w)
    elif t.k == "rec":
        for _, ft in t.fields:
            widths_of(ft, acc)
    elif t.k == "arr":
        widths_of(t.elem, acc)
    acc.update({1, 8, 16})
    return acc


def render_tb(g, top, cycles):
    rng = random.Random(g.seed ^ 0x5EED)
    ins = [o for m, o in top.ports if m == "in" and o.name not in ("clk0", "clk1", "rst", "arst")]
    outs = [o for o in top.sigs if o.name in top.out_names]
    edge_ins = [o for o in ins if rng.random() < 0.25]
    main_ins = [o for o in ins if o not in edge_ins]
    ws = set()
    for o in ins:
        widths_of(o.t, ws)
    rv = "\n".join(f"    variable r{w} : std_ulogic_vector({w - 1} downto 0);" for w in sorted(ws))
    draw = """    variable s : unsigned(63 downto 0) := x"%016X";
    variable l : line;
%s
    procedure draw(v : out std_ulogic_vector) is
      variable t : std_ulogic_vector(v'length - 1 downto 0);
      variable k : natural := 0;
    begin
      while k < v'length loop
        s := s xor shift_left(s, 13);
        s := s xor shift_right(s, 7);
        s := s xor shift_left(s, 17);
        for j in 0 to 63 loop
          exit when k = v'length;
          t(k) := s(j);
          k := k + 1;
        end loop;
      end loop;
      v := t;
    end procedure;"""
    sig = [f"  signal {o.name} : {o.t.mark()} := {zero(o.t)};" for o in ins + outs]
    hold = lambda o: [f"draw(r8); if unsigned(r8) > {rng.choice([40, 90, 160])} then"] + ["  " + x for x in draw_stmts(o.name, o.t, 0)] + ["end if;"]
    drive = [x for o in main_ins for x in hold(o)]
    pr = ["write(l, integer'image(c));"] + [f"write(l, string'(\" {o.name}=\") & {img(o.name, o.t)});" for o in outs] + ["writeline(output, l);"]
    rst = ["draw(r8); if unsigned(r8) < 20 or c < 2 then rst <= '1'; else rst <= '0'; end if;",
           "draw(r8); if unsigned(r8) < 8 then arst <= '1'; elsif c > 0 then arst <= '0'; end if;"]
    maps = [f"      {o.name} => {o.name}" for _, o in top.ports]
    I = lambda xs, n: "\n".join(ind(n) + x for x in xs)
    if rng.random() < 0.5:  # both clocks from one process with one wait: their edges come in one delta
        clocks = """  clocks : process
    variable t : natural := 0;
  begin
    wait for 1 ns;
    t := t + 1;
    if t mod 5 = 0 then
      clk0 <= not clk0;
    end if;
    if t mod 7 = 0 then
      clk1 <= not clk1;
    end if;
  end process;"""
        n_tb = 3
    else:
        clocks = "  clk0 <= not clk0 after 5 ns;\n  clk1 <= not clk1 after 7 ns;"
        n_tb = 4
    return f"""-- tbprocs={n_tb}
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use std.textio.all;
use work.fz_pkg.all;
entity fz_tb is
end entity;
architecture a of fz_tb is
  signal clk0, clk1 : std_ulogic := '0';
  signal rst, arst : std_ulogic := '1';
{chr(10).join(sig)}
begin
{clocks}
  dut : entity work.fz_top
    generic map (G_K => {rng.randint(0, 15)}, G_B => {rng.choice(['true', 'false'])})
    port map (
{(',' + chr(10)).join(maps)});
  main : process
{draw % (rng.getrandbits(63) | 1, rv)}
  begin
    wait for 6500 ps;
    for c in 0 to {cycles - 1} loop
{I(pr, 3)}
{I(rst + drive, 3)}
      wait for 2 ns;
{I(pr, 3)}
{I([x for o in main_ins if rng.random() < 0.4 for x in hold(o)], 3)}
      wait for 8 ns;
    end loop;
    std.env.finish;
  end process;
  edge : process
{draw % (rng.getrandbits(63) | 1, rv)}
  begin
    wait until rising_edge(clk0);
{I([x for o in edge_ins for x in hold(o)] or ['null;'], 2)}
  end process;
end architecture;
"""


def finish(g, top):
    """Decide the out ports and constant values (after generation, before rendering)."""
    rng = random.Random(g.seed ^ 0xC0)
    for c in g.pconsts:
        c.v = g.const_text(c.t)
    for ent in g.ents:
        for c in ent.consts:
            c.v = g.const_text(c.t)


def build(seed, scale=1.0):
    g = Gen(seed, scale)
    top = g.design()
    finish(g, top)
    return g


def render(g, cycles=40):
    top = next(e for e in g.ents if e.name == "fz_top")
    parts = [render_pkg(g)]
    for ent in reversed(g.ents):
        parts.append(render_ent(g, ent, ent is top))
    parts.append(render_tb(g, top, cycles))
    return "\n\n".join(parts) + "\n"


# ---------------------------------------------------------------- running
SUMMARY = re.compile(r"cycle mode: (\d+) of (\d+) processes")


def mem_ok():
    for l in open("/proc/meminfo"):
        if l.startswith("MemAvailable"):
            return int(l.split()[1]) // 1048576 >= 20
    return False


def script(binary, tiers, warm, checks):
    """bash steps of one design in its directory: analysis, elaboration, then the runs; each
    run's stdout, stderr and status in files named after it."""
    y = f"Y={binary}; S='--std=2008 --work=work:lib'; "
    st = ['$Y analyze $S d.vhd > a.out 2>&1 || exit 97', '$Y elab $S fz_tb > e.out 2>&1 || exit 98',
          '$Y run $S --cycle-report fz_tb > report.out 2> report.err']
    runs = []
    for t in tiers:
        runs.append((f"normal-{t}", f"--tier={t}", ""))
        runs.append((f"cycle-{t}", f"--tier={t} --cycle", ""))
        if warm:
            runs.append((f"warm-{t}", f"--tier={t} --cycle", ""))  # the region from the cache of the run before
    if checks:
        runs.append(("checks", f"--tier={tiers[0]} --cycle", "YAVS_CYCLE_CHECKS=all YAVS_CYCLE_SKIP_CHECK=1 YAVS_CYCLE_IMPORT_CHECK=1 YAVS_CYCLE_GUARD=all "))
    for name, flags, env in runs:
        if name.startswith("cycle-"):  # cold: no region code from an earlier run
            st.append("rm -rf lib/yavs-cache/cycle")
        st.append(f"{env}$Y run $S {flags} fz_tb > {name}.out 2> {name}.err; echo $? > {name}.rc")
        if name.startswith("cycle-"):
            st.append(f"ls lib/yavs-cache/cycle/cycle-*.img > {name}.cached 2>/dev/null")
    return y + "; ".join(st), [r[0] for r in runs]


META = re.compile(r"metavalue|'[UXWZ-]'|=[0-9A-F,]*[UXWZ]")
T0_META = re.compile(r"^\*\* Warning: 0fs\+\d+: NUMERIC_STD\.\S+: metavalue detected")


def norm(out):
    """stdout without the metavalue warnings of time 0: an expression actual of a port is 'U'
    until its equivalent process first runs, and cycle-based processes give no ieee warnings."""
    lines, keep = out.splitlines(), []
    i = 0
    while i < len(lines):
        if T0_META.match(lines[i]):
            i += 2 if i + 1 < len(lines) and lines[i + 1].startswith("    Process ") else 1
            continue
        keep.append(lines[i])
        i += 1
    return "\n".join(keep)


def strip(err):
    return "\n".join(l for l in err.splitlines() if not l.startswith("cycle mode: "))


def one(seed, a, binary):
    d = Path(a.out) / f"s{seed}"
    shutil.rmtree(d, ignore_errors=True)
    d.mkdir(parents=True)
    g = build(seed, a.scale)
    (d / "d.vhd").write_text(render(g, a.cycles))
    return check(d, a, binary, seed)


def check(d, a, binary, seed=None):
    tiers = a.tiers.split(",")
    checks = a.checks and (seed is None or seed % a.checks == 0)
    sh, runs = script(binary, tiers, a.warm, checks)
    env = dict(os.environ, YAVS_STD_CACHE=str(Path(a.out) / "std-cache"))
    cmd = ["timeout", "300", "systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=2G", "bash", "-c", sh]
    p = subprocess.run(cmd, cwd=d, env=env, capture_output=True, text=True)
    res = {"seed": seed, "dir": str(d), "verdict": "same", "why": "", "gen": GEN}
    if p.returncode in (97, 98, 124):
        res["verdict"] = {97: "analysis", 98: "elab", 124: "timeout"}[p.returncode]
        f = {97: "a.out", 98: "e.out"}.get(p.returncode)
        res["why"] = (d / f).read_text()[:2000] if f else ""
        return res
    rd = lambda n, x: ((lambda t: norm(t) if x == "out" else t)((d / f"{n}.{x}").read_text(errors="replace"))
                       if (d / f"{n}.{x}").exists() else "")
    m = SUMMARY.search(rd("report", "err"))
    tb = re.search(r"-- tbprocs=(\d+)", (d / "d.vhd").read_text())
    res["members"], res["procs"] = (int(m.group(1)), int(m.group(2)) - int(tb.group(1))) if m else (0, 0)
    res["reasons"] = reasons(rd("report", "err"))
    res["tags"] = tags(d, rd("report", "err"))
    for t in tiers:
        ref = (rd(f"normal-{t}", "rc"), rd(f"normal-{t}", "out"), rd(f"normal-{t}", "err"))
        if ref[0].strip() != "0":
            res.update(verdict="normal-fails", why=f"normal {t} exit {ref[0].strip()}: " + ref[2][-1500:])
            return res
        if t != tiers[0]:
            r0 = rd(f"normal-{tiers[0]}", "out")
            if ref[1] != r0:
                res.update(verdict="differs", why=f"normal {t} differs from normal {tiers[0]}", run=f"normal-{t}")
                return res
        if a.warm and res["members"] and not rd(f"cycle-{t}", "cached").strip():
            res.update(verdict="differs", run=f"warm-{t}", why="the cold run left no region code in the cache: the warm run is not warm")
            return res
        for n in [r for r in runs if r.endswith("-" + t) and not r.startswith("normal")] + (["checks"] if t == tiers[0] and "checks" in runs else []):
            got = (rd(n, "rc"), rd(n, "out"), rd(n, "err"))
            if got[0] != ref[0] or got[1] != ref[1] or strip(got[2]) != strip(ref[2]):
                res.update(verdict="differs", run=n, why=first_diff(ref, got))
                return res
    if res["members"] == 0:
        res["verdict"] = "vacuous"
    return res


def first_diff(ref, got):
    if ref[0] != got[0]:
        return f"exit {ref[0].strip()} normal, {got[0].strip()} cycle; cycle stderr: " + got[2][-1500:]
    for x, y, what in ((ref[1], got[1], "stdout"), (strip(ref[2]), strip(got[2]), "stderr")):
        if x != y:
            a, b = x.splitlines(), y.splitlines()
            for i, (u, v) in enumerate(zip(a + [""] * len(b), b + [""] * len(a))):
                if u != v:
                    return f"{what} line {i + 1}:\n normal: {u[:600]}\n cycle:  {v[:600]}"
    return "?"


TAGS = {k: re.compile(v) for k, v in {
    "shift/rotate": r"shift_(left|right)|rotate_", "sll/srl/rol/ror": r"\b(sll|srl|rol|ror)\b", "multiply": r" \* ",
    "divide": r"or to_(un)?signed\(-?1,", "resize": r"resize\(", "concat": r"'\([^;]* & ", "constant table": r"ROM\d+\(",
    "constant": r"\b(L?K\d+)\b", "record field": r"\.f\d", "run-time index": r"\(\d* ?\+? ?to_integer\(",
    "for loop": r"\bfor i\d+ in", "exit/next": r"\b(exit|next) when", "case": r"\bcase\b", "function": r"\bfn\d+\(",
    "procedure": r"\bpr\d+\(", "aggregate choice": r"=> [^;]*, others =>", "async reset": r"if arst = '1'",
    "sync reset": r"if rst = '1'", "wait until": r"wait until", "falling edge": r"falling_edge", "'event form": r"'event",
    "clk1": r"\bclk1\b", "variable": r"\bvariable ", "integer": r"integer range|to_integer", "enumeration": r"\bE\d+_\d+\b",
    "signed": r"signed\(", "conditional": r" when .* else ", "selected": r"\bwith .* select", "for generate": r"generate",
    "when edge": r"when (rising|falling)_edge", "b2l": r"b2l\(", "?=": r"\?/?=", "reduction": r"\((and|or|xor) [a-z]",
}.items()}


def tags(d, rep):
    """tag -> [members, processes] of the design's processes holding each construct."""
    text = (d / "d.vhd").read_text().splitlines()
    out, on = {}, None
    for l in rep.splitlines():
        if re.match(r"^  \S", l):
            on = "member" if l.strip().startswith("cycle-based,") else "out" if "stay in the event kernel" in l else None
        m = on and re.match(r"^\s+fz_tb\.dut\S* .*?\(d\.vhd:(\d+)\)", l)
        if not m:
            continue
        i = int(m.group(1)) - 1
        j = i
        while j < len(text) and j < i + 400 and not text[j].strip().startswith(("end process", "end generate")) and (j == i or text[i].lstrip().startswith(("process", "g_"))):
            j += 1
        body = "\n".join(text[i:j + 1])
        for k, rx in TAGS.items():
            if rx.search(body):
                c = out.setdefault(k, [0, 0])
                c[0] += on == "member"
                c[1] += 1
    return out


def reasons(rep):
    """reason -> count of the design's processes that stay in the event kernel (`--cycle-report`)."""
    out, on = {}, False
    for l in rep.splitlines():
        if re.match(r"^  \S", l):
            on = "stay in the event kernel" in l
        elif on and l.startswith("      fz_tb.dut"):
            why = re.sub(r"\s*\(d\.vhd:\d+(:\d+)?\)", "", l.strip().split("  ", 1)[-1].strip())
            why = re.sub(r"`[^`]*`", "`.`", why)
            out[why] = out.get(why, 0) + 1
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    g = sub.add_parser("gen")
    g.add_argument("seed", type=int)
    g.add_argument("--scale", type=float, default=1.0)
    g.add_argument("--cycles", type=int, default=40)
    for name in ("run", "shrink", "check"):
        r = sub.add_parser(name)
        if name == "run":
            r.add_argument("--seeds", default="0:100", help="A:B, the seeds A to B - 1")
        elif name == "shrink":
            r.add_argument("seed", type=int)
            r.add_argument("--reason", help="reduce while the region report holds this text, instead of a difference")
        else:
            r.add_argument("dir", help="a directory with d.vhd (a reproducer edited by hand)")
        r.add_argument("--bin", default=os.environ.get("YAVS_BIN", "yavs"))
        r.add_argument("--out", default=os.path.expanduser("~/.cache/yavs-cyclefuzz"))
        r.add_argument("--jobs", type=int, default=2)
        r.add_argument("--tiers", default="interp,clift")
        r.add_argument("--warm", action="store_true", help="each cycle run again with the region from the cache")
        r.add_argument("--checks", type=int, default=0, help="every Nth seed also with the region's own checks")
        r.add_argument("--scale", type=float, default=1.0)
        r.add_argument("--cycles", type=int, default=40)
        r.add_argument("--keep", action="store_true", help="keep the directories of passing seeds")
        r.add_argument("--json", help="append one line per seed to this file")
        if name == "run":
            r.add_argument("--min-share", type=float, default=60.0, help="fail when fewer percent of the processes joined")
    a = ap.parse_args()
    if a.cmd == "gen":
        print(render(build(a.seed, a.scale), a.cycles), end="")
        return 0
    binary = shutil.which(a.bin) or a.bin
    if a.cmd == "check":
        r = check(Path(a.dir), a, binary)
        print(json.dumps(r, indent=1))
        return 0 if r["verdict"] in ("same", "vacuous") else 1
    if a.cmd == "shrink":
        return shrink(a, binary)
    lo, hi = map(int, a.seeds.split(":"))
    import concurrent.futures as cf
    tot = {"same": 0}
    mem, procs, bad = 0, 0, []
    reasons_all = {}
    jobs = min(a.jobs, 2)
    with cf.ThreadPoolExecutor(jobs) as ex:
        futs = []
        for seed in range(lo, hi):
            while not mem_ok():
                time.sleep(60)
            futs.append(ex.submit(one, seed, a, binary))
            if len(futs) >= jobs * 4:
                futs = report(futs, tot, reasons_all, bad, a)
        report(futs, tot, reasons_all, bad, a, wait=True)
    rows = tot.pop("_rows", [])
    m = sum(r.get("members", 0) for r in rows)
    p = sum(r.get("procs", 0) for r in rows)
    print(f"\n{hi - lo} designs: " + ", ".join(f"{k} {v}" for k, v in sorted(tot.items())))
    print(f"members {m} of {p} design processes ({100 * m / max(p, 1):.1f}%)")
    for k, v in sorted(reasons_all.items(), key=lambda x: -x[1])[:15]:
        print(f"  {v:6}  {k[:150]}")
    tg = {}
    for r in rows:
        for k, (m1, n1) in r.get("tags", {}).items():
            c = tg.setdefault(k, [0, 0])
            c[0] += m1
            c[1] += n1
    print("members by construct (processes holding it): " + ", ".join(f"{k} {m1}/{n1} ({100 * m1 // max(n1, 1)}%)" for k, (m1, n1) in sorted(tg.items(), key=lambda x: x[1][0] / max(x[1][1], 1))))
    for r in bad:
        print(f"{r['verdict']}: seed {r['seed']} {r.get('run', '')} {r['dir']}\n  {r['why'][:800]}")
    # Any seed that did not compare (a design the normal mode refuses is a generator or yavs bug too),
    # and a region that holds fewer of the designs' processes than the floor, fail the run.
    share = 100 * m / max(p, 1)
    if share < a.min_share:
        print(f"members {share:.1f}% of the design processes, under the floor of {a.min_share}%: the run did not test the region")
    return 1 if bad or share < a.min_share else 0


def report(futs, tot, reasons_all, bad, a, wait=False):
    keep = []
    for f in futs:
        if not wait and not f.done():
            keep.append(f)
            continue
        r = f.result()
        tot[r["verdict"]] = tot.get(r["verdict"], 0) + 1
        tot.setdefault("_rows", []).append(r)
        for k, v in r.get("reasons", {}).items():
            reasons_all[k] = reasons_all.get(k, 0) + v
        if a.json:
            with open(a.json, "a") as fh:
                fh.write(json.dumps(r) + "\n")
        if r["verdict"] not in ("same", "vacuous"):
            bad.append(r)
            print(f"{r['verdict']}: seed {r['seed']} {r.get('run', '')}", flush=True)
        elif not a.keep:
            shutil.rmtree(r["dir"], ignore_errors=True)
    return keep


# ---------------------------------------------------------------- the shrinker
def stmt_lists(g):
    """Every statement list of the design, nested ones too, in a fixed order."""
    def rec(lst):
        yield lst
        for st in lst:
            if st.k == "if":
                for _, b in st.a[0]:
                    yield from rec(b)
                if st.a[1] is not None:
                    yield from rec(st.a[1])
            elif st.k == "case":
                for _, b in st.a[1]:
                    yield from rec(b)
            elif st.k == "for":
                yield from rec(st.a[2])
    for ent in g.ents:
        for p in ent.procs:
            yield from rec(p.body)
            yield from rec(p.rst_body)
    for f in g.funcs:
        yield from rec(f["body"])
    for p in g.procs:
        yield from rec(p["body"])


def all_exprs(g):
    out = []
    for lst in stmt_lists(g):
        for st in lst:
            for e in exprs_of(st):
                out += list(e.walk())
    for ent in g.ents:
        for p in ent.procs:
            for k in ("e", "els", "sel", "en"):
                if getattr(p, k, None) is not None:
                    out += list(getattr(p, k).walk())
            for z in [z for a in getattr(p, "arms", None) or [] for z in a if isinstance(z, Ex)]:
                out += list(z.walk())
            for _, e in getattr(p, "maps", None) or []:
                if e is not None:
                    out += list(e.walk())
    for f in g.funcs:
        out += list(f["ret_e"].walk())
    return out


def edits(g):
    """(name, count) of the edit kinds; edit(g, kind, i) applies the i-th of a kind."""
    lists = list(stmt_lists(g))
    return [("proc", sum(len(e.procs) for e in g.ents)),
            ("sig", sum(len(e.sigs) + len(e.ports) for e in g.ents)),
            ("narrow", len(vec_types(g))),
            ("const", len(g.pconsts) + sum(len(e.consts) for e in g.ents) + len(g.funcs) + len(g.procs)),
            ("stmt", sum(len(l) for l in lists)),
            ("hoist", sum(len(l) for l in lists)),
            ("ex", len(all_exprs(g))),
            ("kid", len(all_exprs(g)))]


def vec_types(g):
    """The vector types of signals, ports and variables, each once (objects share them)."""
    out, seen = [], set()
    objs = [o for e in g.ents for o in e.sigs + [p[1] for p in e.ports]]
    objs += [v for e in g.ents for p in e.procs for v in p.decls]
    for o in objs:
        if o.t.k in VEC and o.t.w > 1 and id(o.t) not in seen:
            seen.add(id(o.t))
            out.append(o.t)
    return out


def edit(g, kind, i):
    """Apply edit i of a kind to g in place; False if it does not apply."""
    if kind == "narrow":  # half the width: a slice now outside fails analysis, a mismatch shows as a metavalue
        t = vec_types(g)[i]
        t.w = max(1, t.w // 2)
        t.hi = t.lo + t.w - 1
        return True
    if kind == "proc":
        for e in g.ents:
            if i < len(e.procs):
                del e.procs[i]
                return True
            i -= len(e.procs)
        return False
    if kind == "const":  # a constant or subprogram nothing uses any more
        for lst in [g.pconsts] + [e.consts for e in g.ents] + [g.funcs, g.procs]:
            if i < len(lst):
                del lst[i]
                return True
            i -= len(lst)
        return False
    if kind == "sig":  # a signal or port nothing reads any more (else analysis fails)
        for e in g.ents:
            if i < len(e.sigs):
                o = e.sigs.pop(i)
                e.ports = [p for p in e.ports if p[1] is not o]
                e.out_names.discard(o.name)
                return True
            i -= len(e.sigs)
            if i < len(e.ports):
                if e.ports[i][1].name in ("clk0", "clk1", "rst", "arst") or e.ports[i][0] == "out":
                    return False
                del e.ports[i]
                return True
            i -= len(e.ports)
        return False
    if kind in ("stmt", "hoist"):
        for lst in stmt_lists(g):
            if i < len(lst):
                if kind == "stmt":
                    del lst[i]
                    return True
                st = lst[i]
                body = st.a[0][0][1] if st.k == "if" else next((b for _, b in st.a[1] if b), None) if st.k == "case" else st.a[2] if st.k == "for" else None
                if body is None or st.k == "for":  # a for body uses its index
                    return False
                lst[i:i + 1] = body
                return True
            i -= len(lst)
        return False
    e = all_exprs(g)[i]
    if kind == "ex":
        z = "false" if e.t.k == "bool" else zero(e.t) if e.t.k != "int" or e.r is None or e.r[0] <= 0 <= e.r[1] else str(e.r[0])
        if e.a == [] and e.f == z:
            return False
        e.f, e.a = z, []
        return True
    for k in e.a:
        if k.t.k == e.t.k and (e.t.k not in VEC or k.t.w == e.t.w) and (e.t.k not in ("enum", "rec", "arr") or k.t.name == e.t.name):
            if e.t.k == "int" and (k.r is None or e.r is None or k.r[0] < e.r[0] or k.r[1] > e.r[1]):
                continue
            e.f, e.a, e.r = k.f, k.a, k.r
            return True
    return False


def shrink(a, binary):
    import copy
    seed = a.seed
    g = build(seed, a.scale)
    d = Path(a.out) / f"shrink{seed}"
    shutil.rmtree(d, ignore_errors=True)
    d.mkdir(parents=True)
    tiers, run = a.tiers.split(","), None
    cycles = a.cycles

    def test(g2, cyc):
        nonlocal run
        try:
            text = render(g2, cyc)
        except Exception:
            return False
        (d / "d.vhd").write_text(text)
        for x in d.iterdir():
            if x.name != "d.vhd":
                shutil.rmtree(x) if x.is_dir() else x.unlink()
        r = check(d, a, binary, None)
        if a.reason:  # a reason of the region report, for a checker or lowering gap
            rep = d / "report.err"
            return r if rep.exists() and a.reason in rep.read_text(errors="replace") else False
        if r["verdict"] == "differs" and (run is None or r["run"] == run):
            rr = r["run"]
            normal = (d / f"normal-{rr.split('-')[-1] if rr != 'checks' else a.tiers.split(',')[0]}.out").read_text(errors="replace")
            if META.search(norm(normal)):  # a narrowed width made a length mismatch: not the same bug
                return False
            run = r["run"]
            return r
        return False

    if a.reason:
        run = "report"
    r = test(g, cycles)
    if not r:
        print(f"seed {seed} does not differ", file=sys.stderr)
        return 1
    t = run.split("-")[-1] if run not in ("checks", "report") else tiers[0]
    a.tiers, a.warm, a.checks = t, run.startswith("warm"), 1 if run == "checks" else 0
    m = re.search(r"stdout line (\d+)", r["why"] or "")
    if m:  # two lines a sample: stop just after the first that differs
        c = int(m.group(1)) // 2 + 2
        if c < cycles and test(g, c):
            cycles = c
    tries = 0
    changed = True
    while changed:
        changed = False
        for kind, _ in edits(g):
            i = 0
            while i < dict(edits(g))[kind]:
                g2 = copy.deepcopy(g)
                tries += 1
                if edit(g2, kind, i) and test(g2, cycles):
                    g, changed = g2, True
                    print(f"  {kind} {i}: {len(render(g, cycles).splitlines())} lines", file=sys.stderr, flush=True)
                else:
                    i += 1
    final = render(g, cycles)
    (d / "d.vhd").write_text(final)
    r = check(d, a, binary, None)
    print(f"{tries} tries; {len(final.splitlines())} lines in {d / 'd.vhd'} ({run})\n{r['why']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
