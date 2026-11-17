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

sys.path.insert(0, str(Path(__file__).resolve().parent))
import cyclegate as G  # noqa: E402  (the contract classes)

GEN = hashlib.sha1(Path(__file__).read_bytes()).hexdigest()[:10]  # the generator a seed's design came from

WIDTHS = [1, 1, 2, 3, 4, 5, 7, 8, 8, 12, 16, 31, 32, 33, 63, 64, 64, 65, 65, 100, 127, 128, 128, 129]
BIG = 1 << 30
# Constructs a seed may use, each switched on per seed (--feat sets them); the row and the `-- fz:` comments
# of the design name them. rtslice: run-time slices; md: arrays of two dimensions; bigmem: memories of more than
# 64 entries; ucon: records and arrays constrained by a subtype; caseq: `case?`, `select?`, `?=` with '-';
# impure: impure functions reading signals; sigparam: signal parameters and concurrent procedure calls;
# ifgen: if-generate; cgen: composite generics; gwidth: one entity at several widths; assert: asserts and
# reports; zpass: a 'Z' of the testbench passed on, and a metavalue read only when valid; inout: a tri-state
# pin with a pull; dclk: clocks made by the design; xout: concurrent assignments `after 0 ns` (outside the region in
# both contracts, on paths between clocks) and clocks gated twice. view (VHDL-2019 seeds only, drawn apart so that
# other seeds keep their designs): concurrent calls through a mode view and its converse.
FEATS = ["rtslice", "md", "bigmem", "ucon", "alias", "while", "caseq", "impure", "sigparam", "ifgen", "cgen",
         "gwidth", "assert", "zpass", "inout", "dclk", "xout", "view"]
NOT93 = {"ucon", "caseq"}  # VHDL-2008 only
MSG = "fzmsg"  # the text of the region's asserts and reports, filtered from both outputs (cycle mode drops them)


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
            return "std_logic" if getattr(s, "res", False) else "std_ulogic"
        if s.k in VEC:
            base = {"slv": "std_ulogic_vector", "uns": "unsigned", "sgn": "signed"}[s.k]
            return f"{base}({s.lo} to {s.hi})" if s.asc else f"{base}({s.hi} downto {s.lo})"
        if s.k == "int":
            return f"integer range {s.lo} to {s.hi}"
        return s.name

    def n(s):  # elements of an array (rows of arr2), values of an enum
        return s.hi - s.lo + 1 if s.k in ("arr", "arr2") + VEC else len(s.lits)

    def dim2(s):  # the second index range of arr2, as a one-dimensional array type
        return T("arr", name=None, elem=s.elem, lo=s.lo2, hi=s.hi2, asc=s.asc2)

    def same(s, o):
        if s.k != o.k:
            return False
        if s.k in VEC:
            return s.w == o.w
        if s.k == "int":
            return o.lo >= s.lo and o.hi <= s.hi
        return s.k == "sl" or s.name == o.name


VEC = ("slv", "uns", "sgn")
COMP = ("rec", "arr", "arr2")


class Ex:
    """An expression: `f` with `{}` holes filled by `a` (each an Ex). `t` its type, `r` an integer
    range, `sig` the signal a name reads."""

    __slots__ = ("f", "a", "t", "r", "sig", "xs")

    def __init__(s, f, t, a=(), r=None, sig=None, xs=()):
        s.f, s.t, s.a, s.r, s.sig, s.xs = f, t, list(a), r, sig, list(xs)  # xs: signals an impure call reads

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
    if t.k == "arr2":
        return f"{t.name}'(others => (others => {zero(t.elem)}))"
    return f"{t.name}'(others => {zero(t.elem)})"


def bits(rng, w):
    return "".join(rng.choice("01") for _ in range(w))


class Obj:
    __slots__ = ("name", "t", "cls", "v", "init", "base", "alias")  # cls: sig, var, const, gen, loop

    def __init__(s, name, t, cls, base=None, alias=None):
        s.name, s.t, s.cls, s.v, s.init = name, t, cls, None, None
        s.base, s.alias = base or name, alias  # an alias: the signal it names, and the text it aliases


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
        s.aliases, s.impures, s.dclks, s.special, s.raw = [], [], [], {}, None


# ---------------------------------------------------------------- the generator
class Gen:
    def __init__(s, seed, scale=1.0, feat=None, std=None):
        s.rng = random.Random(seed)
        s.seed, s.scale = seed, scale
        s.feat = {f for f in FEATS if f != "view" and s.rng.random() < 0.3} if feat is None else set(feat)
        r = s.rng.random()
        s.std = std or ("1993" if r < 0.1 else "2019" if r < 0.2 else "2008")
        if feat is None and random.Random(f"view{seed}").random() < 0.5:
            s.feat.add("view")
        if s.std != "2019":
            s.feat.discard("view")
        s.v93 = s.std == "1993"
        if s.v93:
            s.feat -= NOT93
        s.used, s.extra, s.sprocs, s.gw, s.in_actual = set(), [], [], None, False
        s.views = []
        s.ent = None
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
            s.recs.append(T("rec", name=f"r{i}_t", fields=fs, uc="ucon" in s.feat and s.chance(0.6) and any(ft.k in VEC for _, ft in fs)))
        for i in range(s.rng.randint(0, 4)):
            r = s.rng.random()
            el = s.vtype(w=s.width(70)) if r < 0.55 else s.rng.choice(s.recs) if r < 0.7 and s.recs else s.scalar_type()
            if el.k == "arr":
                el = T("sl")
            n = s.rng.choice([1, 2, 4, 4, 8, 16, 5])
            lo = s.rng.choice([0, 0, 0, 1, 4])
            s.arrs.append(T("arr", name=f"a{i}_t", elem=el, lo=lo, hi=lo + n - 1, asc=s.chance(0.8),
                            uc="ucon" in s.feat and el.k in VEC and s.chance(0.6)))
        big = "bigmem" in s.feat
        for _ in range(2 if big and s.chance(0.5) else 1):
            if s.chance(0.8 if big else 0.3):  # a bigger memory
                hi = s.rng.choice([31, 63] + ([64, 65, 100, 127, 128, 255, 511] if big else []))
                lo = s.rng.choice([0, 0, 1, 3]) if big else 0
                s.arrs.append(T("arr", name=f"a{len(s.arrs)}_t", elem=s.vtype(w=s.width(70)), lo=lo, hi=lo + hi, asc=not big or s.chance(0.7),
                                uc="ucon" in s.feat and s.chance(0.4)))
        if "md" in s.feat:  # arrays of two dimensions
            for _ in range(s.rng.randint(1, 2)):
                r = s.rng.random()
                el = s.vtype(w=s.width(70)) if r < 0.6 else T("sl") if r < 0.8 else s.scalar_type()
                if el.k == "arr":
                    el = T("sl")
                lo, lo2 = s.rng.choice([0, 0, 1, 2]), s.rng.choice([0, 0, 1, 5])
                s.arrs.append(T("arr2", name=f"md{len(s.arrs)}_t", elem=el, lo=lo, hi=lo + s.rng.randint(0, 4), asc=s.chance(0.7),
                                lo2=lo2, hi2=lo2 + s.rng.randint(0, 3), asc2=s.chance(0.6)))
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
        if t.k == "arr2":
            if rng.random() < 0.3 or t.n() == 1 or t.dim2().n() == 1:
                return f"(others => (others => {s.const_text(t.elem)}))"
            return "(" + ", ".join("(" + ", ".join(s.const_text(t.elem) for _ in range(t.dim2().n())) + ")" for _ in range(t.n())) + ")"
        if t.k == "arr":
            if rng.random() < 0.2:
                return f"(others => {s.const_text(t.elem)})"
            return "(" + ", ".join(s.const_text(t.elem) for _ in range(t.n())) + ")" if t.n() > 1 else f"(others => {s.const_text(t.elem)})"
        return s.lit(t).text()

    # ------------------------------------------------------------ names
    def descend(s, o, write=False):
        """A random path into object o: a list of (Ex, type) from the whole object down."""
        e = Ex(o.name, o.t, sig=o.base if o.cls == "sig" else None,
               r=(o.t.lo, o.t.hi) if o.t.k == "int" else None)
        if o.cls == "gen" and o.t.k == "int":
            e.r = o.t.r
        if o.alias:
            s.used.add("alias")
        if o.cls == "gen" and o.t.k not in ("int", "bool"):
            s.used.add("cgen")
        out = [(e, o.t)]
        t = o.t
        for _ in range(3):
            if getattr(t, "uc", False):
                s.used.add("ucon")
            if t.k == "rec" and s.chance(0.8):
                f, ft = s.rng.choice(t.fields)
                e = Ex("{}." + f, ft, [e])
                t = ft
            elif t.k == "arr" and s.chance(0.8):
                ix = s.index(t, write)
                e = Ex("{}({})", t.elem, [e, ix])
                t = t.elem
            elif t.k == "arr2" and s.chance(0.8):
                s.used.add("md")
                e = Ex("{}({}, {})", t.elem, [e, s.index(t, write), s.index(t.dim2(), write)])
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
        if n > 64:
            s.used.add("bigmem")
        if n > 1 and s.chance(0.15) and s.depth > 0:
            s.depth -= 1
            try:
                return s.int_in(t.lo, t.hi)
            finally:
                s.depth += 1
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
                e = s.slice(e, u, w)
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
            if r < 0.6 and not s.v93:  # numeric_std of 2008
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

    def slice(s, e, u, w):
        """A slice of w elements of e (of vector type u): static, or from an index known at run time."""
        if "rtslice" in s.feat and s.depth >= 0 and s.chance(0.5):
            s.used.add("rtslice")
            d = s.depth
            s.depth = min(d, 1)
            try:
                k = s.int_in(u.lo, u.hi - w + 1)
            finally:
                s.depth = d
            return Ex(f"{{}}({{}} to {{}} + {w - 1})" if u.asc else f"{{}}({{}} + {w - 1} downto {{}})", T(u.k, w=w), [e, k, k])
        a = s.rng.randint(u.lo, u.hi - w + 1)
        return Ex(f"{{}}({a} to {a + w - 1})" if u.asc else f"{{}}({a + w - 1} downto {a})", T(u.k, w=w), [e])

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
            ops += ["gate", "gate", "not", "b2l", "b2l"] + ([] if s.v93 else ["red", "match"])
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
            if "caseq" in s.feat and s.chance(0.5):  # a constant with '-' elements: a mask
                s.used.add("caseq")
                pat = "".join(rng.choice("01--") for _ in range(w))
                return Ex(rng.choice(["({} ?= \"%s\")", "({} ?/= \"%s\")"]) % pat, t, [s.vec("slv", w)])
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
            op = rng.choice(["+", "-", "*", "mod", "rem", "/", "abs"] + ([] if s.v93 else ["min", "max"]))
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
            if t.k == "arr2":
                return Ex(f"{t.name}'(others => (others => {{}}))", t, [s.expr(t.elem)])
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
        """A call of a function of the package returning t, or of an impure one of the architecture."""
        rng = s.rng
        # ponytail: not in port actuals, which elaboration still rejects (E2106); drop the guard once fixed.
        if "impure" in s.feat and not s.in_func and not s.in_actual and s.chance(0.5):
            mine = {x.base for x in s.read if x.cls == "sig"}
            c = [f for f in s.ent.impures if f["ret"].k == t.k and f["ret"].w == t.w and set(f["sigs"]) <= mine]
            f = rng.choice(c) if c and s.chance(0.7) else s.make_impure(t)
            if f:
                s.used.add("impure")
                args = "(" + ", ".join("{}" for _ in f["params"]) + ")" if f["params"] else ""
                return Ex(f["name"] + args, t, [s.expr(p.t) for p in f["params"]], xs=f["sigs"])
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
        extra, s.extra = s.extra, []
        try:
            f["body"] = s.stmts(rng.randint(1, 4), 2, var_only=True)
            f["ret_e"] = s.expr(rt)
            locs += s.extra
        finally:
            s.read, s.write, s.depth, s.loopv, s.in_func = saved
            s.extra = extra
        s.funcs.append(f)
        return f

    def make_impure(s, ret):
        """An impure function of the architecture reading signals the caller may read, or None."""
        rng = s.rng
        sigs = [o for o in s.read if o.cls == "sig" and o.v != "local"]  # not an alias of a process
        if not sigs:
            return None
        params = [Obj(f"p{i}", s.scalar_type(), "var") for i in range(rng.randint(0, 2))]
        f = {"name": s.fresh("ifn"), "params": params, "ret": T(ret.k, w=ret.w), "locs": []}
        saved = (s.read, s.write, s.depth, s.loopv, s.used, s.lconsts)
        s.read, s.write, s.depth, s.loopv, s.used, s.lconsts = params + sigs, [], 2, [], set(), []
        try:
            f["ret_e"] = s.expr(f["ret"])
            if s.chance(0.6):  # read one signal for sure
                o = rng.choice(sigs)
                f["ret_e"] = s.mix(f["ret_e"], o)
        finally:
            s.read, s.write, s.depth, s.loopv, s.used, s.lconsts = saved
        f["sigs"] = sorted({x.sig for x in f["ret_e"].walk() if x.sig} | {y for x in f["ret_e"].walk() for y in x.xs})
        f["body"] = []
        s.ent.impures.append(f)
        return f

    def mix(s, e, o):
        """e (a vector, logic or integer) combined with something of signal o."""
        t = e.t
        if t.k in VEC:
            c = [x for x in s.descend(o) if x[1].k in VEC]
            if c:
                x, u = c[-1]
                return Ex("({} xor {})", t, [e, s.conv(Ex(f"resize(unsigned({{}}), {t.w})", T("uns", w=t.w), [x]), "uns", t.k, t.w)])
        if t.k == "sl" and o.t.k == "sl":
            return Ex("({} xor {})", t, [e, Ex(o.name, o.t, sig=o.base)])
        return e

    def sproc_call(s, seq):
        """A call of a procedure of the package with signal parameters: in a process (seq), or (not seq)
        the outputs as fresh signals for a concurrent call. Returns the St, or (text, outs) when not seq."""
        rng = s.rng
        ins = [o for o in s.read if o.cls == "sig" and o.t.k not in ("int",)]
        if not ins:
            return None
        p = rng.choice(s.sprocs) if s.sprocs and s.chance(0.6) else None
        if p is None:
            a = rng.choice(ins)
            p = s.make_sproc(a.t)
        args = []
        for kind, x in p["params"]:
            if kind == "sig":
                c = [o for o in s.read if o.cls == "sig" and o.t.same(x.t) and x.t.same(o.t)]
                if not c:
                    return None
                o = rng.choice(c)
                args.append(Ex(o.name, o.t, sig=o.base))
            else:
                s.depth = 2
                args.append(s.expr(x.t))
        outs = []
        for y in p["outs"]:
            if seq:
                c = [o for o in s.write if o.cls == "sig" and not o.alias and o.t.same(y.t) and y.t.same(o.t)]
                if not c:
                    return None
                o = rng.choice(c)
                outs.append(Ex(o.name, o.t, sig=o.name))
            else:
                o = Obj(s.fresh("c"), y.t, "sig")
                outs.append(o)
        s.used.add("sigparam")
        if seq:
            return St("call", p["name"], args + outs)
        return p["name"], args, outs

    def view_calls(s, ent, avail, sigs):
        """A record signal whose x a concurrent call drives through a mode view (x out, y in) that reads
        y; y comes from a concurrent assignment, or from a call through the converse view (which the
        subset checker takes for reading x, a loop: that pair stays out)."""
        rng = s.rng
        ins = [o for o in avail if o.cls == "sig" and o.t.k != "int"]
        if not ins:
            return False
        i = len(s.views)
        vt = T("rec", name=f"vw{i}_t", fields=[("x", s.scalar_type()), ("y", s.scalar_type())])
        a1, a2 = rng.choice(ins), rng.choice(ins)
        procs, conv = [], s.chance(0.3)
        for k, a, tgt, rd in (("a", a1, "x", ["y"]), ("b", a2, "y", []))[: 2 if conv else 1]:
            saved = (s.read, s.write, s.depth, s.loopv, s.in_func, s.used)
            ft = dict(vt.fields)
            s.read = [Obj("a", a.t, "sig")] + [Obj(f"m.{f}", ft[f], "sig") for f in rd] + s.pconsts
            s.write, s.depth, s.loopv, s.in_func, s.used = [], 2, [], True, set()
            try:
                e = s.expr(ft[tgt])
            finally:
                s.read, s.write, s.depth, s.loopv, s.in_func, s.used = saved
            procs.append((f"vp{i}{k}", a, k, tgt, e))
        s.views.append({"t": vt, "procs": [(n, a.t, k, tgt, e) for n, a, k, tgt, e in procs]})
        v = Obj(s.fresh("v"), vt, "sig")
        for n, a, *_ in procs:
            ent.procs.append(Proc("ccall", name=n, args=[Ex(a.name, a.t, sig=a.base)], outs=[v], used={"view"}))
        if not conv:
            s.read, s.depth, s.used = [o for o in avail], rng.randint(1, 3), {"view"}
            y = Obj(f"{v.name}.y", dict(vt.fields)["y"], "sig")
            ent.procs.append(Proc("conc", tgt=y, e=s.expr(y.t), used=s.used))
        avail.append(v)
        sigs.append(v)
        return True

    def make_sproc(s, t):
        """A package procedure: signal and constant inputs, signal outputs assigned on every path."""
        rng = s.rng
        name = s.fresh("sp")
        params = [("sig", Obj("a0", t, "sig"))]
        for i in range(1, rng.randint(1, 3)):
            k = rng.choice(["sig", "const"])
            params.append((k, Obj(f"a{i}", s.scalar_type(), "sig" if k == "sig" else "var")))
        outs = [Obj(f"y{i}", s.scalar_type(), "sig") for i in range(rng.randint(1, 2))]
        saved = (s.read, s.write, s.depth, s.loopv, s.in_func, s.used)
        s.read, s.write, s.depth, s.loopv, s.in_func, s.used = [x for _, x in params] + s.pconsts, outs, 2, [], True, set()
        extra, s.extra = s.extra, []
        try:
            body = [St("assign", Ex(o.name, o.t, sig=o.name), "<=", s.expr(o.t)) for o in outs]
            body += s.stmts(rng.randint(0, 2), 2)  # outs of mode out are not read
            locs = s.extra
        finally:
            s.read, s.write, s.depth, s.loopv, s.in_func, s.used = saved
            s.extra = extra
        p = {"name": name, "params": params, "outs": outs, "body": body, "locs": locs}
        s.sprocs.append(p)
        return p

    def make_proc(s):
        rng = s.rng
        name = s.fresh("pr")
        params = [Obj(f"a{i}", s.scalar_type(), "var") for i in range(rng.randint(1, 2))]
        outs = [Obj(f"y{i}", s.scalar_type(), "var") for i in range(rng.randint(1, 2))]
        saved = (s.read, s.write, s.depth, s.loopv, s.in_func)
        # VHDL-1993 reads no `out` parameter
        s.read, s.write, s.depth, s.loopv, s.in_func = params + ([] if s.v93 else outs) + s.pconsts, outs, 2, [], True
        extra, s.extra = s.extra, []
        try:
            body = [St("assign", Ex(o.name, o.t), ":=", s.expr(o.t)) for o in outs]
            body += s.stmts(rng.randint(0, 3), 2, var_only=True)
            locs = s.extra
        finally:
            s.read, s.write, s.depth, s.loopv, s.in_func = saved
            s.extra = extra
        p = {"name": name, "params": params, "outs": outs, "body": body, "locs": locs}
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
                s.depth = 1
                e = s.slice(e, t, w)
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
        if d > 0 and "while" in s.feat and r < 0.36:
            return s.while_stmt(d, var_only)
        if "assert" in s.feat and not s.in_func and r < 0.37:  # dropped in cycle-based processes; filtered (MSG)
            s.used.add("assert")
            msg = Ex(f'"{MSG}{s.fresh("")}"', T("str"))
            if s.chance(0.5):
                msg = Ex(msg.f + " & integer'image({})", T("str"), [s.int_raw()]) if s.chance(0.5) else Ex(msg.f + " & std_ulogic'image({})", T("str"), [s.sl()])
            sev = rng.choice(["note", "warning"])
            return St("assert", s.cond() if s.chance(0.7) else None, msg, sev)
        if "sigparam" in s.feat and not var_only and not s.in_func and r < 0.39 and any(o.cls == "sig" and not o.alias for o in s.write):
            c = s.sproc_call(seq=True)
            if c:
                return c
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
        if r < 0.45 and t.k not in COMP and not s.v93:
            arms = [(s.expr(t), s.cond()) for _ in range(rng.randint(1, 2))]
            return St("cassign", e, op, arms, s.expr(t))
        if t.k in VEC and not e.a and t.w > 1 and s.chance(0.15):  # an aggregate with named or range choices
            a = rng.randint(t.lo, t.hi)
            b = rng.randint(a, t.hi)
            rng_txt = f"{a} to {b}" if t.asc else f"{b} downto {a}"
            if b > a and s.v93:  # a range choice of elements (slices are VHDL-2008)
                return St("assign", e, op, Ex(f"({rng_txt} => {{}}, others => {{}})", t, [s.sl(), s.sl()]))
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
        if c and not s.v93:
            sel = rng.choice(c)[0]
        elif c:  # 1993: the selector's subtype must be locally static
            sel = Ex(f"fzu{w}_t({{}})", T("uns", w=w), [rng.choice(c)[0]])
        else:
            sel = Ex(f"fzu{w}_t'({{}})", T("uns", w=w), [s.vec("uns", w)])
        if "caseq" in s.feat and s.chance(0.5):  # case?: the arms differ in the elements they care about
            s.used.add("caseq")
            care = [i for i in range(w) if s.chance(0.6)] or [0]
            vals = rng.sample(range(1 << len(care)), min(1 << len(care), rng.randint(1, 4)))
            pats = []
            for v in vals:
                b = ["-"] * w
                for j, i in enumerate(care):
                    b[i] = "01"[(v >> j) & 1]
                pats.append('"' + "".join(b) + '"')
            arms = [(x, s.stmts(rng.randint(0, 2), d - 1, var_only)) for x in pats]
            arms.append(("others", s.stmts(rng.randint(0, 1), d - 1, var_only)))
            if sel.t.k != "slv":
                sel = Ex("std_ulogic_vector({})", T("slv", w=w), [sel])
            return St("case", sel, arms, True)
        vals = rng.sample(range(1 << w), min(1 << w, rng.randint(1, 4)))
        arms = [(f'"{v:0{w}b}"', s.stmts(rng.randint(0, 2), d - 1, var_only)) for v in vals]
        arms.append(("others", s.stmts(rng.randint(0, 1), d - 1, var_only)))
        return St("case", sel, arms)

    def while_stmt(s, d, var_only):
        """A while loop (or a loop with an exit) counted by a variable of its own, bounded by 8."""
        rng = s.rng
        s.used.add("while")
        v = Obj(s.fresh("w"), T("int", lo=0, hi=16), "var")
        s.extra.append(v)
        s.depth = 1
        bound = Ex(str(rng.randint(0, 6)), T("int"), r=None) if s.chance(0.4) else s.int_in(0, 8)
        o = Obj(v.name, T("int", lo=0, hi=9), "loop")
        s.read.append(o)
        s.loopv.append(o)
        try:
            body = s.stmts(rng.randint(1, 3), d - 1, var_only)
        finally:
            s.read.remove(o)
            s.loopv.remove(o)
        return St("while", v.name, bound, body, s.chance(0.3))

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
        s.ent = ent
        s.econsts, s.arrs_local, s.lconsts = ent.consts, [], []
        for _ in range(rng.randint(0, 3)):  # constants of any type, for their parts
            (s.pconsts if s.chance(0.4) else s.econsts).append(Obj(s.fresh("K"), s.any_type(), "const"))
        ent.generics = [Obj("G_K", T("int", lo=0, hi=15), "gen"), Obj("G_B", T("bool"), "gen")]
        ent.generics[0].t.r = (0, 15)
        if "cgen" in s.feat:  # generics of composite types, with defaults
            for i in range(rng.randint(1, 2)):
                t = s.vtype(w=s.width(70)) if s.chance(0.6) or not (s.recs or s.arrs) else rng.choice(s.recs + s.arrs)
                o = Obj(f"G_C{i}", t, "gen")
                o.v = s.const_text(t)
                ent.generics.append(o)
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
        if "alias" in s.feat:  # aliases of inputs and registers, read by any process
            for _ in range(rng.randint(1, 3)):
                a = s.alias_of([o for o in data_in + regs if o.t.k in VEC + ("rec",)])
                if a:
                    ent.aliases.append(a)
                    avail.append(a)
        if "dclk" in s.feat:  # clocks made by the design: gated, divided, copied, chosen by a generic
            for _ in range(rng.randint(1, 2)):
                s.used = {"dclk"}
                o = Obj(s.fresh("dk"), T("sl"), "sig")
                k = rng.choice(["gate", "div", "copy", "mux"])
                s.read, s.depth = list(data_in), 1
                if k == "gate":  # the enable from a register of the falling edge: no glitch, no race with data
                    en = Obj(s.fresh("dke"), T("sl"), "sig")
                    ent.procs.append(Proc("whenclk", tgt=en, e=s.sl(), drives=[en], edge="falling_edge(clk0)", used=s.used))
                    sigs.append(en)
                    ent.procs.append(Proc("conc", tgt=o, e=Ex("({} and {})", T("sl"), [Ex("clk0", T("sl"), sig="clk0"), Ex(en.name, T("sl"), sig=en.name)]),
                                          used=s.used))
                elif k == "div":
                    ent.procs.append(Proc("whenclk", tgt=o, e=Ex(f"(not {o.name})", T("sl"), sig=o.name), drives=[o], edge="rising_edge(clk0)", used=s.used))
                elif k == "copy":
                    ent.procs.append(Proc("conc", tgt=o, e=Ex(rng.choice(["clk0", "clk1"]), T("sl")), used=s.used))
                else:
                    ent.procs.append(Proc("cond", tgt=o, arms=[(Ex("clk0", T("sl")), Ex("G_B", T("bool")))], els=Ex("clk1", T("sl")), used=s.used))
                ent.dclks.append(o.name)
                sigs.append(o)
        if top and "xout" in s.feat:  # a clk0 register, a member, a process outside, a register on clk0 gated twice
            s.used = {"xout"}
            en = Obj(s.fresh("xe"), T("sl"), "sig")
            g1, g2 = Obj(s.fresh("xg"), T("sl"), "sig"), Obj(s.fresh("xg"), T("sl"), "sig")
            t = T("uns", w=rng.choice(WIDTHS))
            r, m1, m2, q = (Obj(s.fresh(p), t, "sig") for p in ("xr", "xm", "xo", "xq"))
            ent.procs += [Proc("whenclk", tgt=en, e=Ex("'1'", T("sl")), drives=[en], edge="falling_edge(clk0)", used=s.used),
                          Proc("conc", tgt=g1, e=Ex(f"(clk0 and {en.name})", T("sl")), used=s.used),
                          Proc("conc", tgt=g2, e=Ex(f"({g1.name} and {en.name})", T("sl")), used=s.used),
                          Proc("whenclk", tgt=r, e=Ex(f"({r.name} + 1)", t), drives=[r], edge="rising_edge(clk0)", used=s.used),
                          Proc("conc", tgt=m1, e=Ex(f"(not {r.name})", t), used=s.used),
                          Proc("conc", tgt=m2, e=Ex(m1.name, t), used=s.used, late=True),
                          Proc("whenclk", tgt=q, e=Ex(f"({q.name} + {m2.name})", t), drives=[q], edge=f"rising_edge({g2.name})", used=s.used)]
            sigs += [en, g1, g2, r, m1, m2, q]
        if top and "zpass" in s.feat:
            s.zpass(ent, avail, sigs)
        if top and "inout" in s.feat:
            s.inout(ent, avail, sigs, regs + data_in)
        # combinational logic in order: each reads inputs, registers and earlier logic
        for _ in range(max(1, int(rng.randint(2, 7) * s.scale))):
            r = rng.random()
            s.used = set()
            if r < 0.15 and depth > 0:
                outs = s.instance(ent, depth, avail)
                avail += outs
                sigs += outs
                continue
            if "gwidth" in s.feat and r < 0.22:
                outs = s.gw_inst(ent, avail)
                avail += outs
                sigs += outs
                continue
            outs = [Obj(s.fresh("c"), s.any_type(), "sig") for _ in range(rng.randint(1, 2))]
            s.read = [o for o in avail]
            if "view" in s.feat and r < 0.3 and s.view_calls(ent, avail, sigs):
                continue
            if "sigparam" in s.feat and r < 0.3:
                c = s.sproc_call(seq=False)
                if c:
                    ent.procs.append(Proc("ccall", name=c[0], args=c[1], outs=c[2], used=s.used))
                    avail += c[2]
                    sigs += c[2]
                    continue
            if "ifgen" in s.feat and r < 0.38:
                o = outs[0]
                s.ifgen(ent, o)
                avail.append(o)
                sigs.append(o)
                continue
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
            s.used = set()
            if len(g) == 1 and g[0] not in mems and s.chance(0.15):  # q <= d when rising_edge(clk);
                s.depth = rng.randint(1, 3)
                ent.procs.append(Proc("whenclk", tgt=g[0], e=s.expr(g[0].t), drives=g,
                                      edge=rng.choice(["rising_edge", "falling_edge"]) + "(" + s.clock() + ")", used=s.used))
                continue
            s.clocked(ent, g, mems)
        ent.sigs = sigs
        names = [o.name for o in sigs if o.name not in ent.special]
        ent.out_names = set(names) if top else set(n for n in names if rng.random() < 0.6) or set(names[:1])
        ent.ports += [("out", o) for o in sigs if o.name in ent.out_names]
        return ent

    def clock(s):
        if s.ent.dclks and s.chance(0.3):
            s.used.add("dclk")
            return s.rng.choice(s.ent.dclks)
        return "clk1" if s.chance(0.15) else "clk0"

    def alias_of(s, objs):
        """An alias of part of one of objs: a slice under a subtype of its own range, or a record field."""
        rng = s.rng
        if not objs:
            return None
        o = rng.choice(objs)
        name = s.fresh("al")
        if o.t.k == "rec":
            f, ft = rng.choice(o.t.fields)
            return Obj(name, ft, "sig" if o.cls == "sig" else o.cls, base=o.base, alias=f"{o.name}.{f}")
        w = rng.randint(1, o.t.w)
        a = rng.randint(o.t.lo, o.t.hi - w + 1)
        part = f"{o.name}({a} to {a + w - 1})" if o.t.asc else f"{o.name}({a + w - 1} downto {a})"
        if w == o.t.w and s.chance(0.5):
            part = o.name
        t = T(o.t.k, w=w, lo=rng.choice([0, 0, 2, 5]), asc=s.chance(0.5))
        return Obj(name, t, "sig" if o.cls == "sig" else o.cls, base=o.base, alias=part)

    def actual(s, ent, e, extra, t=None):
        """e as a port actual (of formal type t): in VHDL-1993 a signal of its own when it is not a name."""
        if not s.v93 or (not e.a and e.sig):
            return e
        o = Obj(s.fresh("c"), t or e.t, "sig")
        ent.procs.append(Proc("conc", tgt=o, e=e))
        extra.append(o)
        return Ex(o.name, o.t, sig=o.name)

    def ifgen(s, ent, o):
        """o assigned in an if-generate on the generics, one arm with a signal of its own."""
        rng = s.rng
        s.used.add("ifgen")
        conds = ["G_B", f"G_K > {rng.randint(0, 15)}", "not G_B", f"G_K = {rng.randint(0, 3)}"]
        arms = []
        for i in range(1 if s.v93 else rng.randint(1, 3)):
            s.depth = rng.randint(1, 3)
            arms.append((rng.choice(conds), s.expr(o.t)))
        s.depth = rng.randint(1, 3)
        els = s.expr(o.t)
        loc = Obj(s.fresh("t"), o.t, "sig") if s.chance(0.4) else None
        ent.procs.append(Proc("ifgen", tgt=o, arms=arms, els=els, loc=loc, label=s.fresh("ig"), used=s.used, v93=s.v93))

    def zpass(s, ent, avail, sigs):
        """A 'Z' of the testbench passed on to outputs, and data with metavalues read only when valid."""
        rng = s.rng
        s.used = {"zpass"}
        zt = T("sl") if s.chance(0.5) else T("slv", w=rng.randint(2, 9))
        zi = Obj(s.fresh("zi"), zt, "sig")
        ent.ports.append(("in", zi))
        ent.special[zi.name] = "z"
        s.read, s.depth = list(avail), 1
        for k in rng.sample(["copy", "mux", "reg"], rng.randint(1, 3)):
            o = Obj(s.fresh("zo"), zt, "sig")
            if k == "copy":
                ent.procs.append(Proc("conc", tgt=o, e=Ex(zi.name, zt, sig=zi.name), used=s.used))
            elif k == "mux":
                ent.procs.append(Proc("cond", tgt=o, arms=[(Ex(zi.name, zt, sig=zi.name), s.cond())], els=s.expr(zt), used=s.used))
            else:
                ent.procs.append(Proc("whenclk", tgt=o, e=Ex(zi.name, zt, sig=zi.name), drives=[o], edge="rising_edge(clk0)", used=s.used))
            sigs.append(o)
        xt = s.vtype("slv", rng.choice([1, 4, 8, 64, 65]))
        xv, xd = Obj(s.fresh("xv"), T("sl"), "sig"), Obj(s.fresh("xd"), xt, "sig")
        ent.ports += [("in", xv), ("in", xd)]
        ent.special[xv.name], ent.special[xd.name] = "xv", "xd:" + xv.name
        q = Obj(s.fresh("xq"), xt, "sig")
        ent.procs.append(Proc("raw", used=s.used, drives=[q], text=f"process (clk0)\nbegin\n  if rising_edge(clk0) then\n    if {xv.name} = '1' then\n"
                              f"      {q.name} <= {xd.name};\n    end if;\n  end if;\nend process;", reads=[xv.name, xd.name]))
        sigs.append(q)
        avail.append(q)

    def inout(s, ent, avail, sigs, srcs):
        """A tri-state pin the testbench pulls to 'H' or 'L': driven when enabled, read back with to_x01."""
        s.used = {"inout"}
        io = Obj(s.fresh("io"), T("sl", res=True), "sig")
        ent.ports.append(("inout", io))
        ent.special[io.name] = "io"
        s.read, s.depth = list(srcs), 2
        d, oe = s.sl(), s.sl()
        ent.procs.append(Proc("raw", used=s.used, drives=[], reads=[], text=f"{io.name} <= {d.text()} when {oe.text()} = '1' else 'Z';"))
        r = Obj(s.fresh("ior"), T("sl"), "sig")
        ent.procs.append(Proc("conc", tgt=r, e=Ex(f"to_x01({io.name})", T("sl"), sig=io.name), used=s.used))
        sigs.append(r)
        avail.append(r)

    def gw_inst(s, ent, avail):
        """Instances of the entity with a width generic, at several widths."""
        saved, s.in_actual = s.in_actual, True
        try:
            return s.gw_inst1(ent, avail)
        finally:
            s.in_actual = saved

    def gw_inst1(s, ent, avail):
        rng = s.rng
        if s.gw is None:
            s.gw = Ent("fz_gw")
            s.gw.raw = gw_text(rng, s.v93)
            s.ents.append(s.gw)
        outs = []
        for w in rng.sample([1, 2, 7, 31, 63, 64, 65, 100, 128, 129], rng.randint(1, 3)):
            s.read = list(avail)
            maps = [("clk0", Ex(s.clock(), T("sl")))]
            for n in ("a", "b"):
                s.depth = rng.randint(0, 2)
                maps.append((n, s.actual(ent, s.vec("uns", w), outs, T("uns", w=w))))
            s.depth = 1
            maps.append(("s", s.actual(ent, s.sl(), outs, T("sl"))))
            for n, t in (("y", T("uns", w=w)), ("z", T("sl")), ("p", T("uns", w=2 * w))):
                if s.chance(0.1):
                    maps.append((n, None))
                    continue
                x = Obj(s.fresh("c"), t, "sig")
                maps.append((n, Ex(x.name, t, sig=x.name)))
                outs.append(x)
            p = Proc("inst", sub="fz_gw", gmap=[("G_W", str(w))], maps=maps)
            p.label = s.fresh("u")
            ent.procs.append(p)
        return outs

    def instance(s, ent, depth, avail):
        try:
            return s.instance1(ent, depth, avail)
        finally:
            s.in_actual = False

    def instance1(s, ent, depth, avail):
        rng = s.rng
        saved = (s.econsts, s.arrs_local, s.read, s.lconsts, s.ent)
        sub = s.entity(depth - 1)
        s.econsts, s.arrs_local, s.read, s.lconsts, s.ent = saved
        s.in_actual = True  # the port maps (after the entity, whose own processes may call impure functions)
        outs_all = []
        for k in range(2 if s.chance(0.3) else 1):
            s.read = list(avail)
            gk = rng.randint(0, 15)
            maps = []
            for mode, o in sub.ports:
                if o.name in ("clk0", "clk1", "rst", "arst"):
                    c = o.name
                    if o.name.startswith("clk") and s.chance(0.1):
                        c = rng.choice(["clk0", "clk1"])
                    elif o.name.startswith("clk") and ent.dclks and s.chance(0.2):
                        c = rng.choice(ent.dclks)
                    maps.append((o.name, Ex(c, o.t, sig=c)))
                    continue
                if mode == "in":
                    s.depth = rng.randint(0, 2)
                    if o.t.k in VEC and o.t.w >= 2 and s.chance(0.25):
                        a = rng.randint(1, o.t.w - 1)
                        tl, th = T(o.t.k, w=a), T(o.t.k, w=o.t.w - a)
                        lo = o.t.lo
                        p1 = f"{o.name}({lo + a - 1} downto {lo})" if not o.t.asc else f"{o.name}({lo} to {lo + a - 1})"
                        p2 = f"{o.name}({o.t.hi} downto {lo + a})" if not o.t.asc else f"{o.name}({lo + a} to {o.t.hi})"
                        maps.append((p1, s.actual(ent, s.vec(o.t.k, a), outs_all, T(o.t.k, w=a))))
                        maps.append((p2, s.actual(ent, s.vec(o.t.k, o.t.w - a), outs_all, T(o.t.k, w=o.t.w - a))))
                    else:
                        c = s.names(lambda u: u.k == o.t.k and u.name == o.t.name, s.read) if o.t.k in COMP else []
                        maps.append((o.name, s.actual(ent, rng.choice(c)[0] if c and s.chance(0.8) else s.expr(o.t), outs_all, o.t)))
                else:
                    # A clock is no data: logic of a clock sampled at that clock's edge is a race.
                    if s.chance(0.08) or o.name.startswith("dk"):
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
            gmap = [("G_K", str(gk)), ("G_B", rng.choice(["true", "false"]))]
            gmap += [(g.name, s.const_text(g.t)) for g in sub.generics[2:] if s.chance(0.7)]
            p = Proc("inst", sub=sub.name, gmap=gmap, maps=maps)
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
        s.loopv, s.extra = [], []
        s.depth = 2
        body = [St("assign", Ex(o.name, o.t), ":=" if o.cls == "var" else "<=", s.expr(o.t)) for o in vs + outs]
        s.read = s.read + vs  # only after the default assignments: no variable read before it is assigned
        body += s.stmts(rng.randint(1, 5), 2)
        if outs[0].t.k == "rec" and s.chance(0.5):  # the two-process record style
            v = Obj(s.fresh("v"), outs[0].t, "var")
            vs.append(v)
            body = [St("assign", Ex(v.name, v.t), ":=", s.expr(v.t))] + body + [St("assign", Ex(outs[0].name, v.t), "<=", Ex(v.name, v.t))]
        p = Proc("comb", decls=lc + vs + s.extra, body=body, drives=outs, all=s.chance(0.6) and not s.v93, used=s.used)
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
            ent.procs.append(Proc("comb", decls=[], body=body, drives=outs, all=not s.v93))
        elif k == "state":
            v = Obj(s.fresh("v"), o.t, "var")
            s.read = s.read + [v]
            body = [St("assign", Ex(o.name, o.t), "<=", s.expr(o.t)), St("assign", Ex(v.name, v.t), ":=", s.expr(v.t))]
            ent.procs.append(Proc("comb", decls=[v], body=body, drives=outs, all=not s.v93))
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
        if r < 0.5 or out.t.k in COMP:
            ent.procs.append(Proc("conc", tgt=out, e=s.expr(out.t), used=s.used, late="xout" in s.feat and s.chance(0.4)))
        elif r < 0.75:
            arms = [(s.expr(out.t), s.cond()) for _ in range(rng.randint(1, 3))]
            ent.procs.append(Proc("cond", tgt=out, arms=arms, els=s.expr(out.t), used=s.used))
        elif "caseq" in s.feat and r < 0.82:  # select? with '-' choices
            s.used.add("caseq")
            w = rng.randint(1, 4)
            care = [i for i in range(w) if s.chance(0.6)] or [0]
            vals = rng.sample(range(1 << len(care)), min(1 << len(care), rng.randint(1, 3)))
            arms = []
            for v in vals:
                b = ["-"] * w
                for j, i in enumerate(care):
                    b[i] = "01"[(v >> j) & 1]
                arms.append((s.expr(out.t), '"' + "".join(b) + '"'))
            ent.procs.append(Proc("sel", tgt=out, sel=s.vec("slv", w), arms=arms, els=s.expr(out.t), q=True, used=s.used))
        elif r < 0.9 or out.t.k not in VEC:
            et = rng.choice(s.enums)
            sel = s.expr(et)
            lits = et.lits[:]
            rng.shuffle(lits)
            k = rng.randint(1, len(lits))
            arms = [(s.expr(out.t), lit) for lit in lits[:k]]
            ent.procs.append(Proc("sel", tgt=out, sel=sel, arms=arms, els=s.expr(out.t) if k < len(lits) else None, used=s.used))
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
            ent.procs.append(Proc("gen_bits", tgt=out, e=e, used=s.used))

    def clocked(s, ent, regs, mems):
        rng = s.rng
        clk = s.clock()
        edge = rng.choice(["rising_edge({c})", "rising_edge({c})", "falling_edge({c})", "{c}'event and {c} = '1'", "{c} = '1' and {c}'event"]).format(c=clk)
        reset = rng.choice(["sync", "sync", "async", "none"])
        lc = s.local_consts()
        vs = [Obj(s.fresh("v"), s.any_type(), "var") for _ in range(rng.randint(0, 2))]
        als = []
        if "alias" in s.feat and s.chance(0.6):  # aliases of part of a register or a variable, written and read
            a = s.alias_of([o for o in regs + vs if o.t.k in VEC + ("rec",) and o not in mems])
            if a:
                a.v = "local"
                als.append(a)
        s.write = regs + vs + als
        s.read = s.read + vs + als
        s.loopv, s.extra = [], []
        body = []
        for m in [x for x in regs if x in mems]:  # a write port at a run-time index
            s.depth = 2
            we = s.cond()
            ix = s.index(m.t)
            if ix.a == []:
                s.depth = 1
                ix = s.index(m.t)
            tg = Ex("{}({}, {})", m.t.elem, [Ex(m.name, m.t, sig=m.name), ix, s.index(m.t.dim2())]) if m.t.k == "arr2" else \
                Ex("{}({})", m.t.elem, [Ex(m.name, m.t, sig=m.name), ix])
            body.append(St("if", [(we, [St("assign", tg, "<=", s.expr(m.t.elem))])], None))
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
        p = Proc("clocked", decls=lc + vs + s.extra + als, body=body, rst_body=rst_body, drives=regs, clk=clk, edge=edge, reset=reset, en=en,
                 shape=shape, rsig=rsig, used=s.used)
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
            q = "?" if len(a) > 2 and a[2] else ""
            out.append(f"{ind(n)}case{q} {a[0].text()} is")
            for ch, body in a[1]:
                out.append(f"{ind(n + 1)}when {ch} =>")
                out += render_stmts(body, n + 2) or [ind(n + 2) + "null;"]
            if a[1][-1][0] != "others" and a[0].t.k != "enum":
                out.append(f"{ind(n + 1)}when others => null;")
            out.append(f"{ind(n)}end case{q};")
        elif k == "while":
            v, bound, body, plain = a
            out.append(f"{ind(n)}{v} := 0;")
            if plain:
                out += [f"{ind(n)}loop", f"{ind(n + 1)}exit when {v} >= {bound.text()};"]
            else:
                out.append(f"{ind(n)}while {v} < {bound.text()} loop")
            out.append(f"{ind(n + 1)}{v} := {v} + 1;")
            out += render_stmts(body, n + 1)
            out.append(f"{ind(n)}end loop;")
        elif k == "assert":
            c, msg, sev = a
            out.append(f"{ind(n)}assert {c.text()} report {msg.text()} severity {sev};" if c else f"{ind(n)}report {msg.text()} severity {sev};")
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
    for e in getattr(p, "args", None) or []:
        yield e


def reads(p):
    out, mine = [], {o.name for o in p.drives}
    for e in proc_exprs(p):
        for x in e.walk():
            for y in ([x.sig] if x.sig else []) + x.xs:
                if y not in out and y not in mine:
                    out.append(y)
    return out


def tagged(line, p):
    """The first line of a process, with the constructs it holds as a comment (for `tags`)."""
    u = getattr(p, "used", None)
    return f"{line}  -- fz: {','.join(sorted(u))}" if u else line


def render_proc(p, n=1):
    # The report names a statement of an if-generate by its own line: tag those too.
    return "\n".join(tagged(l, p) if i == 0 or (p.k == "ifgen" and "<=" in l) else l for i, l in enumerate(render_proc1(p, n).split("\n")))


def decl_text(v):
    if v.alias:
        return f"alias {v.name} is {v.alias};" if "." in v.alias else f"alias {v.name} : {v.t.mark()} is {v.alias};"
    if v.cls == "const":
        return f"constant {v.name} : {v.t.mark()} := {v.v};"
    return f"variable {v.name} : {v.t.mark()} := {zero(v.t)};"


def render_proc1(p, n):
    lab = (p.label + " : ") if p.label else ""
    if p.k == "inst":
        maps = ",\n".join(f"{ind(n + 2)}{f} => {'open' if e is None else e.text()}" for f, e in p.maps)
        gm = ", ".join(f"{k} => {v}" for k, v in p.gmap)
        return (f"{ind(n)}{p.label} : entity work.{p.sub}\n{ind(n + 1)}generic map ({gm})\n"
                f"{ind(n + 1)}port map (\n{maps});")
    if p.k == "raw":
        return "\n".join(ind(n) + l for l in p.text.split("\n"))
    if p.k == "ccall":
        return f"{ind(n)}{p.label or ''}{' : ' if p.label else ''}{p.name}(" + ", ".join([e.text() for e in p.args] + [o.name for o in p.outs]) + ");"
    if p.k == "ifgen":
        out = []
        body = lambda e: ([f"{ind(n + 1)}signal {p.loc.name} : {p.loc.t.mark()};", f"{ind(n)}begin", f"{ind(n + 1)}{p.loc.name} <= {e.text()};",
                           f"{ind(n + 1)}{p.tgt.name} <= {p.loc.name};"] + ([] if p.v93 else [f"{ind(n)}end;"])) if p.loc else [f"{ind(n + 1)}{p.tgt.name} <= {e.text()};"]
        if p.v93:  # two generates of opposite conditions
            c, e = p.arms[0]
            return "\n".join([f"{ind(n)}{p.label}a : if {c} generate"] + body(e) + [f"{ind(n)}end generate;", f"{ind(n)}{p.label}b : if not ({c}) generate",
                              f"{ind(n + 1)}{p.tgt.name} <= {p.els.text()};", f"{ind(n)}end generate;"])
        for i, (c, e) in enumerate(p.arms):
            out.append(f"{ind(n)}{p.label} : if {c} generate" if i == 0 else f"{ind(n)}elsif {c} generate")
            out += body(e) if i == 0 else [f"{ind(n + 1)}{p.tgt.name} <= {e.text()};"]
        out += [f"{ind(n)}else generate", f"{ind(n + 1)}{p.tgt.name} <= {p.els.text()};", f"{ind(n)}end generate;"]
        return "\n".join(out)
    if p.k == "conc":
        return f"{ind(n)}{lab}{p.tgt.name} <= {p.e.text()}{' after 0 ns' if getattr(p, 'late', False) else ''};"
    if p.k == "whenclk":
        return f"{ind(n)}{lab}{p.tgt.name} <= {p.e.text()} when {p.edge};"
    if p.k == "cond":
        arms = " else ".join(f"{e.text()} when {c.text()}" for e, c in p.arms)
        return f"{ind(n)}{lab}{p.tgt.name} <= {arms} else {p.els.text()};"
    if p.k == "sel":
        arms = [f"{e.text()} when {c}" for e, c in p.arms]
        if p.els is not None:
            arms.append(f"{p.els.text()} when others")
        q = "?" if getattr(p, "q", False) else ""
        return f"{ind(n)}{lab}with {p.sel.text()} select{q} {p.tgt.name} <=\n{ind(n + 2)}" + (",\n" + ind(n + 2)).join(arms) + ";"
    if p.k == "gen_bits":
        g = p.label or "g_" + p.tgt.name
        return (f"{ind(n)}{g}_gen : for gi in {p.tgt.name}'range generate\n{ind(n + 1)}{p.tgt.name}(gi) <= {p.e.text()};\n"
                f"{ind(n)}end generate;")
    decls = [ind(n + 1) + decl_text(v) for v in p.decls]
    if p.k == "waits":
        return "\n".join([f"{ind(n)}{lab}process"] + decls + [f"{ind(n)}begin", f"{ind(n + 1)}wait until rising_edge({p.clk});",
                          f"{ind(n + 1)}wait for 1 ns;"] + render_stmts(p.body, n + 1) + [f"{ind(n)}end process;"])
    if p.k == "comb":
        rd = getattr(p, "sens", None) or reads(p)
        if getattr(p, "partial", False):
            rd = rd[:1]
        sens = "all" if p.all else ", ".join(rd) or ("clk0" if V93[0] else "all")  # VHDL-1993 has no `all`
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


def gw_text(rng, v93):
    """The entity with a width generic G_W: a random choice of operations that hold for every width."""
    ys = ["a + b", "a xor b", "not a", "a - r", "rotate_left(a, 1)", "shift_right(b, G_W / 2)", "resize(a * b, G_W)",
          "(a(G_W - 1 downto G_W / 2) & b(G_W / 2 downto 1))", "(b sll 1) or (a srl 1)", "r + 1", "a + to_unsigned(G_W, G_W)",
          "a / (b or to_unsigned(1, G_W))", "a rem (b or to_unsigned(1, G_W))", "unsigned(std_ulogic_vector(a) xnor std_ulogic_vector(r))",
          "rotate_right(r, to_integer(b(0 downto 0)))", "resize(shift_left(resize(a, 2 * G_W), G_W), G_W)"]
    zs = ["a(G_W - 1)", "a(to_integer(resize(b, 7)) mod G_W)", "b2l(a = r)", "r(0) xor s", "b2l(a > b)", "c(G_W / 2)"] + ([] if v93 else ["xor std_ulogic_vector(a)"])
    ps = ["a * b", "resize(a, 2 * G_W) + b", "a & b", "r & c",
          "shift_left(resize(a, 2 * G_W), G_W / 2 + 1)", "resize(r, 2 * G_W) - c"]
    o = "buffer" if v93 else "out"
    loop = ["      for i in 0 to G_W - 1 loop", "        v(i) := v(i) xor a(G_W - 1 - i);", "      end loop;"] if rng.random() < 0.6 else []
    if v93:
        gen = ["  g1 : if G_W > 64 generate", f"    c2 <= {rng.choice(ys)};", "  end generate;", "  g2 : if G_W <= 64 generate",
               f"    c2 <= {rng.choice(ys)};", "  end generate;"]
    else:
        gen = ["  g1 : if G_W > 64 generate", f"    c2 <= {rng.choice(ys)};", "  elsif G_W = 1 generate", f"    c2 <= {rng.choice(ys)};",
               "  else generate", f"    c2 <= {rng.choice(ys)};", "  end generate;"]
    return "\n".join([HEAD + "entity fz_gw is", "  generic (G_W : positive := 8);",
        f"  port (clk0 : in std_ulogic; a, b : in unsigned(G_W - 1 downto 0); s : in std_ulogic;",
        f"        y : {o} unsigned(G_W - 1 downto 0) := (others => '0'); z : {o} std_ulogic := '0';",
        f"        p : {o} unsigned(2 * G_W - 1 downto 0) := (others => '0'));", "end entity;", "architecture a of fz_gw is",
        "  signal r, c, c2 : unsigned(G_W - 1 downto 0) := (others => '0');", "begin",
        f"  c <= {rng.choice(ys)} when s = '1' else {rng.choice(ys)};  -- fz: gwidth"] + gen + [
        "  process (clk0)  -- fz: gwidth", "    variable v : unsigned(G_W - 1 downto 0);", "  begin", "    if rising_edge(clk0) then",
        f"      r <= {rng.choice(ys)};", f"      v := {rng.choice(ys)};"] + loop + [
        "      y <= v xor c xor c2;", f"      z <= {rng.choice(zs)};", f"      p <= {rng.choice(ps)};", "    end if;", "  end process;", "end architecture;"])


HEAD = "library ieee;\nuse ieee.std_logic_1164.all;\nuse ieee.numeric_std.all;\nuse work.fz_pkg.all;\n"


def render_pkg(g):
    out = ["library ieee;", "use ieee.std_logic_1164.all;", "use ieee.numeric_std.all;", "package fz_pkg is"]
    for e in g.enums:
        out.append(f"  type {e.name} is ({', '.join(e.lits)});")
    declared = set()

    base = lambda t: {"slv": "std_ulogic_vector", "uns": "unsigned", "sgn": "signed"}[t.k]
    rng_of = lambda t: f"{t.lo} to {t.hi}" if t.asc else f"{t.hi} downto {t.lo}"

    def decl(t):
        if t.k == "rec" and t.name not in declared:
            for _, ft in t.fields:
                decl(ft)
            declared.add(t.name)
            uc = getattr(t, "uc", False)  # a record of unconstrained vectors, constrained by a subtype
            out.append(f"  type {t.name}{'_b' if uc else ''} is record")
            out.extend(f"    {f} : {base(ft) if uc and ft.k in VEC else ft.mark()};" for f, ft in t.fields)
            out.append("  end record;")
            if uc:
                out.append(f"  subtype {t.name} is {t.name}_b(" + ", ".join(f"{f}({rng_of(ft)})" for f, ft in t.fields if ft.k in VEC) + ");")
        elif t.k in ("arr", "arr2") and t.name not in declared:
            decl(t.elem)
            declared.add(t.name)
            if t.k == "arr2":
                out.append(f"  type {t.name} is array ({rng_of(t)}, {rng_of(t.dim2())}) of {t.elem.mark()};")
            elif getattr(t, "uc", False):  # an array of unconstrained vectors, constrained by a subtype
                out.append(f"  type {t.name}_b is array (natural range <>) of {base(t.elem)};")
                out.append(f"  subtype {t.name} is {t.name}_b({rng_of(t)})({rng_of(t.elem)});")
            else:
                out.append(f"  type {t.name} is array ({rng_of(t)}) of {t.elem.mark()};")
    for t in g.recs + g.arrs + [v["t"] for v in g.views]:
        decl(t)
    for i, v in enumerate(g.views):
        out.append(f"  view vw{i}_v of {v['t'].name} is\n    x : out;\n    y : in;\n  end view;")
    conv = lambda k: "" if k == "a" else "'converse"
    vproc = lambda i, n, at, k: f"procedure {n}(signal a : in {at.mark()}; signal m : view vw{i}_v{conv(k)})"
    for w in range(1, 5):  # selectors of case statements (a locally static subtype, for VHDL-1993)
        out.append(f"  subtype fzu{w}_t is unsigned({w - 1} downto 0);")
    out.append("  function b2l(b : boolean) return std_ulogic;")
    for c in g.pconsts:
        out.append(f"  constant {c.name} : {c.t.mark()} := {c.v};")
    for f in g.funcs:
        out.append(f"  subtype {f['name']}_t is {f['ret'].mark()};")
        out.append(f"  function {f['name']}(" + "; ".join(f"{p.name} : {p.t.mark()}" for p in f["params"]) + f") return {f['name']}_t;")
    sig = lambda p: "; ".join([f"{'signal ' if k == 'sig' else ''}{a.name} : in {a.t.mark()}" for k, a in p["params"]] +
                              [f"signal {y.name} : out {y.t.mark()}" for y in p["outs"]])
    for p in g.procs:
        out.append(f"  procedure {p['name']}(" + "; ".join([f"{a.name} : in {a.t.mark()}" for a in p["params"]] + [f"{y.name} : out {y.t.mark()}" for y in p["outs"]]) + ");")
    for p in g.sprocs:
        out.append(f"  procedure {p['name']}({sig(p)});")
    for i, v in enumerate(g.views):
        out += [f"  {vproc(i, n, at, k)};" for n, at, k, _, _ in v["procs"]]
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
        out += [f"    variable {v.name} : {v.t.mark()} := {zero(v.t)};" for v in p["locs"]]
        out.append("  begin")
        out += render_stmts(p["body"], 2)
        out.append("  end procedure;")
    for p in g.sprocs:
        out.append(f"  procedure {p['name']}({sig(p)}) is")
        out += [f"    variable {v.name} : {v.t.mark()} := {zero(v.t)};" for v in p["locs"]]
        out.append("  begin")
        out += render_stmts(p["body"], 2)
        out.append("  end procedure;")
    for i, v in enumerate(g.views):
        for n, at, k, tgt, e in v["procs"]:
            out += [f"  {vproc(i, n, at, k)} is", "  begin", f"    m.{tgt} <= {e.text()};", "  end procedure;"]
    out.append("end package body;")
    return "\n".join(out)


def init_of(g, o):
    return getattr(o, "init", None) or zero(o.t)


def render_ent(g, ent, top):
    if ent.raw:
        return ent.raw
    ports = []
    for mode, o in ent.ports:
        if mode == "in":
            ports.append(f"    {o.name} : in {o.t.mark()}")
        elif mode == "inout":
            ports.append(f"    {o.name} : inout {o.t.mark()} := 'Z'")
        else:  # VHDL-1993 reads no out port: buffer
            ports.append(f"    {o.name} : {'buffer' if g.v93 else 'out'} {o.t.mark()} := {init_of(g, o)}")
    port_names = {o.name for _, o in ent.ports}
    gens = ["G_K : integer range 0 to 15 := 0", "G_B : boolean := false"] + [f"{o.name} : {o.t.mark()} := {o.v}" for o in ent.generics[2:]]
    lines = [HEAD + f"entity {ent.name} is", f"  generic ({'; '.join(gens)});",
             "  port (\n" + ";\n".join(ports) + ");", "end entity;", f"architecture a of {ent.name} is"]
    for c in ent.consts:
        if c.t.k == "arr" and c.t not in g.arrs:
            rng = f"{c.t.lo} to {c.t.hi}" if c.t.asc else f"{c.t.hi} downto {c.t.lo}"
            lines.append(f"  type {c.t.name} is array ({rng}) of {c.t.elem.mark()};")
        lines.append(f"  constant {c.name} : {c.t.mark()} := {c.v};")
    for o in ent.sigs:
        if o.name not in port_names and o.name not in ent.out_names:
            lines.append(f"  signal {o.name} : {o.t.mark()} := {init_of(g, o)};")
    lines += ["  " + decl_text(a) for a in ent.aliases]
    for f in ent.impures:
        lines.append(f"  subtype {f['name']}_t is {f['ret'].mark()};")
        lines.append(f"  impure function {f['name']}" + ("(" + "; ".join(f"{p.name} : {p.t.mark()}" for p in f["params"]) + ")" if f["params"] else "") +
                     f" return {f['name']}_t is\n  begin\n    return {f['ret_e'].text()};\n  end function;")
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
        return f"hx(std_ulogic_vector({name}))" if V93[0] else f"to_hstring({name})"
    if t.k == "arr2":
        d2 = t.dim2()
        return " & \",\" & ".join(img(f"{name}({i}, {j})", t.elem) for i in range(t.lo, t.hi + 1) for j in range(d2.lo, d2.hi + 1))
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
    if t.k == "arr2":
        d2 = t.dim2()
        return [x for i in range(t.lo, t.hi + 1) for j in range(d2.lo, d2.hi + 1) for x in draw_stmts(f"{tgt}({i}, {j})", t.elem, n)]
    return [x for i in range(t.lo, t.hi + 1) for x in draw_stmts(f"{tgt}({i})", t.elem, n)]


def widths_of(t, acc):
    if t.k in VEC:
        acc.add(t.w)
    elif t.k == "rec":
        for _, ft in t.fields:
            widths_of(ft, acc)
    elif t.k in ("arr", "arr2"):
        widths_of(t.elem, acc)
    acc.update({1, 8, 16})
    return acc


def show(o):
    """write statements printing ` name=value`, at most 64 elements a statement."""
    parts = img(o.name, o.t).split(' & "," & ')
    out = []
    for i in range(0, len(parts), 64):
        head = f'string\'(" {o.name}=")' if i == 0 else 'string\'(",")'
        out.append(f"write(l, {head} & " + ' & "," & '.join(parts[i:i + 64]) + ");")
    return out


def render_tb(g, top, cycles):
    rng = random.Random(g.seed ^ 0x5EED)
    sp = top.special
    ins = [o for m, o in top.ports if m == "in" and o.name not in ("clk0", "clk1", "rst", "arst") and o.name not in sp]
    outs = [o for o in top.sigs if o.name in top.out_names]
    pins = [o for m, o in top.ports if m == "inout"]
    edge_ins = [o for o in ins if rng.random() < 0.25]
    main_ins = [o for o in ins if o not in edge_ins]
    ws = set()
    for o in ins + [o for m, o in top.ports if o.name in sp]:
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
    sig = [f"  signal {o.name} : {o.t.mark()} := {zero(o.t)};" for o in ins + outs + [o for m, o in top.ports if o.name in sp and m == "in"]]
    sig += [f"  signal {o.name} : {o.t.mark()} := 'H';" for o in pins]
    hold = lambda o: [f"draw(r8); if unsigned(r8) > {rng.choice([40, 90, 160])} then"] + ["  " + x for x in draw_stmts(o.name, o.t, 0)] + ["end if;"]
    drive = [x for o in main_ins for x in hold(o)]
    for m, o in top.ports:  # the testbench's own 'Z', metavalues on data not valid, a pull on the pin
        k = sp.get(o.name, "")
        if k == "z":
            z = "'Z'" if o.t.k == "sl" else "(others => 'Z')"
            drive += [f"draw(r8); if unsigned(r8) > 120 then {o.name} <= {z};", "else"] + ["  " + x for x in draw_stmts(o.name, o.t, 0)] + ["end if;"]
            if o.t.k != "sl" and rng.random() < 0.5:
                drive.append(f"draw(r8); if unsigned(r8) > 200 then {o.name}({o.t.lo}) <= 'Z'; end if;")
        elif k.startswith("xd:"):
            v = k[3:]
            bad = rng.choice(["'X'", "'U'", "'W'", "'-'", "'Z'"])
            drive += [f"draw(r8); if unsigned(r8) > 110 then {v} <= '1';"] + ["  " + x for x in draw_stmts(o.name, o.t, 0)] + \
                     [f"else {v} <= '0'; {o.name} <= (others => {bad});", "end if;"]
        elif k == "io":
            drive.append(f"draw(r8); if unsigned(r8) > 128 then {o.name} <= 'H'; else {o.name} <= 'L'; end if;")
    shown = outs + pins
    pr = ["write(l, integer'image(c));"] + [x for o in shown for x in show(o)] + ["writeline(output, l);"]
    rst = ["draw(r8); if unsigned(r8) < 20 or c < 2 then rst <= '1'; else rst <= '0'; end if;",
           "draw(r8); if unsigned(r8) < 8 then arst <= '1'; elsif c > 0 then arst <= '0'; end if;"]
    maps = [f"      {o.name} => {o.name}" for _, o in top.ports]
    I = lambda xs, n: "\n".join(ind(n) + x for x in xs)
    stop = g.v93  # VHDL-1993 has no std.env: the clocks stop and the run ends
    if rng.random() < 0.5:  # both clocks from one process with one wait: their edges come in one delta
        clocks = """  clocks : process
    variable t : natural := 0;
  begin
    wait for 1 ns;%s
    t := t + 1;
    if t mod 5 = 0 then
      clk0 <= not clk0;
    end if;
    if t mod 7 = 0 then
      clk1 <= not clk1;
    end if;
  end process;""" % ("\n    if done then\n      wait;\n    end if;" if stop else "")
        n_tb = 3
    elif stop:
        clocks = "  clk0 <= not clk0 after 5 ns when not done else clk0;\n  clk1 <= not clk1 after 7 ns when not done else clk1;"
        n_tb = 4
    else:
        clocks = "  clk0 <= not clk0 after 5 ns;\n  clk1 <= not clk1 after 7 ns;"
        n_tb = 4
    chk = ""
    # A checker of the top-level unit: evaluated at the clock edge, same messages. Not on a zpass output: its
    # 'Z' reads '0' under the default contract, so the check would decide another way (README "Contract classes").
    checked = [o for o in shown if not o.name.startswith("zo")]
    if "assert" in g.feat and checked:
        o = rng.choice(checked)
        e = {"sl": f"{o.name} = '1'", "int": f"{o.name} > {o.t.lo}"}.get(o.t.k)
        if o.t.k == "enum":
            e = f"{o.name} = {o.t.lits[0]}"
        if o.t.k in VEC:
            e = f"{o.name}({o.t.lo}) = '0'"
        if e:
            chk = f"""  chk : process (clk0)
  begin
    if rising_edge(clk0) then
      assert {e} report "checker " & {img(o.name, o.t)} severity note;
    end if;
  end process;
"""
            n_tb += 1
    hx = """  function hx(v : std_ulogic_vector) return string is
    constant d : string(1 to 17) := "0123456789ABCDEFX";
    variable r : string(1 to (v'length + 3) / 4);
    variable x : std_ulogic_vector(4 * r'length - 1 downto 0) := (others => '0');
    variable k : natural;
  begin
    x(v'length - 1 downto 0) := v;
    for i in r'range loop
      k := 0;
      for j in 3 downto 0 loop
        k := 2 * k;
        if x(4 * (r'length - i) + j) = '1' then
          k := k + 1;
        elsif x(4 * (r'length - i) + j) /= '0' then
          k := 16;  -- a digit with a metavalue is 'X', as to_hstring gives it
          exit;
        end if;
      end loop;
      r(i) := d(k + 1);
    end loop;
    return r;
  end function;
  signal done : boolean := false;
""" if g.v93 else ""
    gm = ", ".join([f"G_K => {rng.randint(0, 15)}", f"G_B => {rng.choice(['true', 'false'])}"] + [f"{o.name} => {g.const_text(o.t)}" for o in top.generics[2:]])
    end = "done <= true;\n    wait;" if stop else "std.env.finish;"
    return f"""-- tbprocs={n_tb} std={g.std} feat={','.join(sorted(g.feat))}
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
{hx}begin
{clocks}
{chk}  dut : entity work.fz_top
    generic map ({gm})
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
    {end}
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


def build(seed, scale=1.0, feat=None, std=None):
    g = Gen(seed, scale, feat, std)
    top = g.design()
    finish(g, top)
    return g


V93 = [False]  # the design being rendered is VHDL-1993 (img)


def render(g, cycles=40):
    top = next(e for e in g.ents if e.name == "fz_top")
    V93[0] = g.v93
    parts = [render_pkg(g)]
    for ent in sorted(reversed(g.ents), key=lambda e: e.raw is None):  # the width entity first: all may use it
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


# The cycle runs compare exactly (`--contract exact`): the normal mode's metavalue rules and copied clocks.
EXACT = "--cycle-exact-meta"
CONTRACT = {"exact": True}


def script(binary, tiers, warm, checks, std="2008"):
    """bash steps of one design in its directory: analysis, elaboration, then the runs; each
    run's stdout, stderr and status in files named after it."""
    x = CONTRACT["exact"]
    y = ("export YAVS_CYCLE_EXACT_CLOCKS=1; " if x else "unset YAVS_CYCLE_EXACT_CLOCKS; ") + f"Y={binary}; S='--std={std} --work=work:lib{' ' + EXACT if x else ''}'; "
    st = ['$Y analyze $S d.vhd > a.out 2>&1 || exit 97', '$Y elab $S fz_tb > e.out 2>&1 || exit 98',
          'YAVS_CYCLE_DOWNSTREAM=down.txt $Y run $S --cycle-report fz_tb > report.out 2> report.err']
    st[:2] = [c.replace("$S", "$A") for c in st[:2]]  # analyze and elab take no cycle flag
    y += f"A='--std={std} --work=work:lib'; "
    runs = []
    for t in tiers:
        runs.append((f"normal-{t}", f"--tier={t}", "N"))
        runs.append((f"cycle-{t}", f"--tier={t} --cycle", ""))
        if warm:
            runs.append((f"warm-{t}", f"--tier={t} --cycle", ""))  # the region from the cache of the run before
    if checks:
        runs.append(("checks", f"--tier={tiers[0]} --cycle", "YAVS_CYCLE_CHECKS=all YAVS_CYCLE_SKIP_CHECK=1 YAVS_CYCLE_IMPORT_CHECK=1 YAVS_CYCLE_GUARD=all "))
    for name, flags, env in runs:
        if name.startswith("cycle-"):  # cold: no region code from an earlier run
            st.append("rm -rf lib/yavs-cache/cycle")
        opts = "$A" if env == "N" else "$S"  # the normal mode takes no cycle flag
        env = "" if env == "N" else env
        st.append(f"{env}$Y run {opts} {flags} fz_tb > {name}.out 2> {name}.err; echo $? > {name}.rc")
        if name.startswith("cycle-"):
            st.append(f"ls lib/yavs-cache/cycle/cycle-*.img > {name}.cached 2>/dev/null")
    return y + "; ".join(st), [r[0] for r in runs]


META = re.compile(r"(\S*)(?:metavalue|='[UXWZ-]'|=[0-9A-F,]*[UXWZ])")
T0_META = re.compile(r"^\*\* Warning: 0fs\+\d+: NUMERIC_STD\.\S+: metavalue detected")


def norm(out):
    """stdout without the metavalue warnings of time 0 (an expression actual of a port is 'U'
    until its equivalent process first runs, and cycle-based processes give no ieee warnings),
    and without the reports of the design's own asserts (dropped in cycle-based processes)."""
    lines, keep = out.splitlines(), []
    i = 0
    while i < len(lines):
        if T0_META.match(lines[i]) or (MSG in lines[i] and lines[i].startswith("** ")):
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
    g = build(seed, a.scale, a.feat, a.std)
    (d / "d.vhd").write_text(render(g, a.cycles))
    return check(d, a, binary, seed)


def check(d, a, binary, seed=None):
    tiers = a.tiers.split(",")
    checks = a.checks and (seed is None or seed % a.checks == 0)
    hd = re.search(r"-- tbprocs=\d+ std=(\d+) feat=(\S*)", (d / "d.vhd").read_text())
    sh, runs = script(binary, tiers, a.warm, checks, hd.group(1) if hd else "2008")
    env = dict(os.environ, YAVS_STD_CACHE=str(Path(a.out) / "std-cache"))
    cmd = ["timeout", "300", "systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=2G", "bash", "-c", sh]
    p = subprocess.run(cmd, cwd=d, env=env, capture_output=True, text=True)
    res = {"seed": seed, "dir": str(d), "verdict": "same", "why": "", "gen": GEN}
    if hd:
        res["std"], res["feat"] = hd.group(1), hd.group(2).split(",") if hd.group(2) else []
    if p.returncode not in (97, 98, 124) and "W1404" in (d / "a.out").read_text(errors="replace"):  # a static index outside its range:
        p.returncode = 97  # an error of the design (the shrinker's narrowing makes them), though only a warning at analysis
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
                ok, cls = explained(ref, got, d, rd("report", "err"))
                if not ok:
                    res.update(verdict="differs", run=n, why=first_diff(ref, got))
                    return res
                res["classes"] = sorted(set(res.get("classes", [])) | set(cls))
    if res.get("classes"):
        res["verdict"] = "contract" if not CONTRACT["exact"] else "documented"
    if res["members"] == 0:
        res["verdict"] = "vacuous"
    return res


def explained(ref, got, d, rep):
    """(whether the contract classes explain every difference of a cycle run `got` from the normal run
    `ref`, the classes used): library metavalue warnings of processes outside that read the region, and
    under the default contract metavalues read two-state (tools/difftest/cyclegate.py, README)."""
    if got[0] != ref[0] or strip(got[2]) != strip(ref[2]):
        return False, []
    down = G.read_down(d / "down.txt")
    (n, a), (c, b) = G.drop_down(ref[1], down), G.drop_down(got[1], down)
    cls = G.lib_class(a, b)
    nl, cl = n.splitlines(), c.splitlines()
    if nl != cl and not CONTRACT["exact"] and G.meta_evidence(got[2], rep.splitlines()):
        cl, k = G.meta_align(nl, cl)
        cls += ["metavalues read as 0 or 1"] if k else []
    return nl == cl, cls


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
        fz = re.search(r"-- fz: (\S+)", text[i]) if i < len(text) else None
        for k in [k for k, rx in TAGS.items() if rx.search(body)] + (fz.group(1).split(",") if fz else []):
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
    feat = lambda x: [f for f in x.split(",") if f] if x != "random" else None
    for name in ("run", "shrink", "check"):
        r = sub.add_parser(name)
        r.add_argument("--feat", type=feat, default=None, help=f"the constructs every seed uses, of {','.join(FEATS)} (default: random per seed)")
        r.add_argument("--std", choices=["1993", "2008", "2019"], help="the VHDL version of every seed (default: random per seed)")
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
        r.add_argument("--contract", choices=("exact", "default"), default="exact",
                       help="exact (default): cycle runs with --cycle-exact-meta and YAVS_CYCLE_EXACT_CLOCKS=1; default: plain --cycle")
        if name == "run":
            r.add_argument("--min-share", type=float, default=60.0, help="fail when fewer percent of the processes joined")
    g.add_argument("--feat", type=feat, default=None)
    g.add_argument("--std", choices=["1993", "2008", "2019"])
    a = ap.parse_args()
    if a.cmd == "gen":
        print(render(build(a.seed, a.scale, a.feat, a.std), a.cycles), end="")
        return 0
    CONTRACT["exact"] = a.contract == "exact"
    binary = shutil.which(a.bin) or a.bin
    if a.cmd == "check":
        r = check(Path(a.dir), a, binary)
        print(json.dumps(r, indent=1))
        return 0 if r["verdict"] in ("same", "vacuous", "contract", "documented") else 1
    for f in a.feat or []:
        if f not in FEATS:
            ap.error(f"unknown construct {f!r} in --feat; known: {', '.join(FEATS)}")
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
        if r["verdict"] not in ("same", "vacuous", "contract", "documented"):
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
            elif st.k in ("for", "while"):
                yield from rec(st.a[2])
    for ent in g.ents:
        for p in ent.procs:
            yield from rec(p.body)
            yield from rec(p.rst_body)
    for f in g.funcs:
        yield from rec(f["body"])
    for p in g.procs + g.sprocs:
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
            ("narrow", len(vec_types(g)) * len(NARROW)),
            ("const", len(g.pconsts) + sum(len(e.consts) for e in g.ents) + len(g.funcs) + len(g.procs) + len(g.sprocs)),
            ("decl", sum(len(e.aliases) + len(e.impures) for e in g.ents)),
            ("stmt", sum(len(l) for l in lists)),
            ("hoist", sum(len(l) for l in lists)),
            ("ex", len(all_exprs(g))),
            ("kid", len(all_exprs(g)))]


NARROW = [lambda w: w // 2, lambda w: w - 1, lambda w: 1, lambda w: 64 if w > 64 else w - 1, lambda w: 8]  # widths a vector may shrink to


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
    if kind == "narrow":  # a smaller width: a slice now outside fails analysis, a mismatch shows as a metavalue
        t = vec_types(g)[i // len(NARROW)]
        w = max(1, NARROW[i % len(NARROW)](t.w))
        if w >= t.w:
            return False
        t.w = w
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
        for lst in [g.pconsts] + [e.consts for e in g.ents] + [g.funcs, g.procs, g.sprocs]:
            if i < len(lst):
                del lst[i]
                return True
            i -= len(lst)
        return False
    if kind == "decl":  # an alias or impure function nothing uses any more
        for lst in [x for e in g.ents for x in (e.aliases, e.impures)]:
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
                if body is None or st.k in ("for", "while"):  # a loop body uses its index
                    return False
                lst[i:i + 1] = body
                return True
            i -= len(lst)
        return False
    e = all_exprs(g)[i]
    if kind == "ex":
        if e.t.k == "str":  # a message: no constant to put there
            return False
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
    g = build(seed, a.scale, a.feat, a.std)
    d = Path(a.out) / f"shrink{seed}"
    shutil.rmtree(d, ignore_errors=True)
    d.mkdir(parents=True)
    tiers, run = a.tiers.split(","), None
    cycles = a.cycles
    meta = None  # the outputs showing metavalues at the start (a 'Z' passed on): no others may appear

    def test(g2, cyc):
        nonlocal run, meta
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
            ms = set(META.findall(norm(normal)))
            if meta is not None and not ms <= meta:  # a narrowed width made a length mismatch: not the same bug
                return False
            meta = ms if meta is None else meta
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
