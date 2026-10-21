#!/usr/bin/env python3
"""Generate the differential testbenches tb/*.vhd.

Operand shapes come from the public package declarations (the signature
tables below were derived from sim1's `--dump`, which prints interfaces only).
Usage: gen.py OUTDIR  (then replace `syna` by `ieee` and add the `-- difftest: top=NAME` header line,
as the checked-in files do)
"""
import sys, os, re

# ---- signature tables (interface only) -------------------------------------
V3 = ["UNSIGNED", "SIGNED"]
def arith_sigs():
    s = []
    def add(op, args, ret): s.append((op, args, ret))
    # + and - : (U,U) (S,S) (U,S) (S,U) (U,I) (I,U) (S,I) (I,S) (U,L) (L,U) (S,L) (L,S)
    pairs = [("UNSIGNED","UNSIGNED"),("SIGNED","SIGNED"),("UNSIGNED","SIGNED"),("SIGNED","UNSIGNED"),
             ("UNSIGNED","INTEGER"),("INTEGER","UNSIGNED"),("SIGNED","INTEGER"),("INTEGER","SIGNED"),
             ("UNSIGNED","STD_ULOGIC"),("STD_ULOGIC","UNSIGNED"),("SIGNED","STD_ULOGIC"),("STD_ULOGIC","SIGNED")]
    ret1 = {("UNSIGNED","UNSIGNED"):"UNSIGNED",("SIGNED","SIGNED"):"SIGNED",("UNSIGNED","SIGNED"):"SIGNED",("SIGNED","UNSIGNED"):"SIGNED",
            ("UNSIGNED","INTEGER"):"UNSIGNED",("INTEGER","UNSIGNED"):"UNSIGNED",("SIGNED","INTEGER"):"SIGNED",("INTEGER","SIGNED"):"SIGNED",
            ("UNSIGNED","STD_ULOGIC"):"UNSIGNED",("STD_ULOGIC","UNSIGNED"):"UNSIGNED",("SIGNED","STD_ULOGIC"):"SIGNED",("STD_ULOGIC","SIGNED"):"SIGNED"}
    for op in ("+", "-"):
        for p in pairs: add(op, p, ret1[p])
        for p in pairs: add(op, p, "STD_LOGIC_VECTOR")
    for op, t in (("+","UNSIGNED"),("+","SIGNED"),("-","SIGNED"),("abs","SIGNED")):
        add(op, (t,), t)
    for op, t in (("+","UNSIGNED"),("+","SIGNED"),("-","SIGNED"),("abs","SIGNED")):
        add(op, (t,), "STD_LOGIC_VECTOR")
    for p, r in ((("UNSIGNED","UNSIGNED"),"UNSIGNED"),(("SIGNED","SIGNED"),"SIGNED"),(("SIGNED","UNSIGNED"),"SIGNED"),(("UNSIGNED","SIGNED"),"SIGNED")):
        add("*", p, r)
    for p, r in ((("UNSIGNED","UNSIGNED"),"UNSIGNED"),(("SIGNED","SIGNED"),"SIGNED"),(("SIGNED","UNSIGNED"),"SIGNED"),(("UNSIGNED","SIGNED"),"SIGNED")):
        add("*", p, "STD_LOGIC_VECTOR")
    cmp_pairs = [("UNSIGNED","UNSIGNED"),("SIGNED","SIGNED"),("UNSIGNED","SIGNED"),("SIGNED","UNSIGNED"),
                 ("UNSIGNED","INTEGER"),("INTEGER","UNSIGNED"),("SIGNED","INTEGER"),("INTEGER","SIGNED")]
    for op in ("<","<=",">",">=","=","/="):
        for p in cmp_pairs: add(op, p, "BOOLEAN")
    for f in ("SHL","SHR"):
        add(f, ("UNSIGNED","UNSIGNED"), "UNSIGNED"); add(f, ("SIGNED","UNSIGNED"), "SIGNED")
    add("CONV_INTEGER", ("INTEGER",), "INTEGER"); add("CONV_INTEGER", ("UNSIGNED",), "INTEGER")
    add("CONV_INTEGER", ("SIGNED",), "INTEGER"); add("CONV_INTEGER", ("STD_ULOGIC",), "SMALL_INT")
    for f, r in (("CONV_UNSIGNED","UNSIGNED"),("CONV_SIGNED","SIGNED"),("CONV_STD_LOGIC_VECTOR","STD_LOGIC_VECTOR")):
        for a in ("INTEGER","UNSIGNED","SIGNED","STD_ULOGIC"): add(f, (a,"SIZE"), r)
    add("EXT", ("STD_LOGIC_VECTOR","SIZE"), "STD_LOGIC_VECTOR")
    add("SXT", ("STD_LOGIC_VECTOR","SIZE"), "STD_LOGIC_VECTOR")
    return s

def slv_sigs(signed):
    S = "STD_LOGIC_VECTOR"; s = []
    def add(op, args, ret): s.append((op, args, ret))
    for op in ("+","-"):
        for a in ((S,S),(S,"INTEGER"),("INTEGER",S),(S,"STD_ULOGIC"),("STD_ULOGIC",S)): add(op, a, S)
    add("+", (S,), S)
    if signed:
        add("-", (S,), S); add("abs", (S,), S)
    add("*", (S,S), S)
    for op in ("<","<=",">",">=","=","/="):
        for a in ((S,S),(S,"INTEGER"),("INTEGER",S)): add(op, a, "BOOLEAN")
    add("SHL", (S,S), S); add("SHR", (S,S), S)
    add("CONV_INTEGER", (S,), "INTEGER")
    return s

# ---- value generation -------------------------------------------------------
INTS = [0,1,-1,2,-2,3,7,8,-8,100,127,128,-128,-129,255,256,-256,1000,-1000,32767,-32768,65535,1048581,16777216,-16777217]
INTS_EDGE = [2147483647, -2147483647, -2147483648, 1073741824, -1073741824, 2147483646]
SIZES = [1,2,3,7,8,9,31,32,33,40]
WIDTHS = "(1,2,7,8,31,32,33)"
NK = 16
CURPKG = None

HEAD = """library ieee;
use ieee.std_logic_1164.all;
library syna;
use syna.std_logic_arith.all;
{extra}
entity {name} is end entity;
architecture t of {name} is
  procedure pr(constant s : in string) is begin report s; end procedure;
  type int_arr is array (natural range <>) of integer;
  constant INTS  : int_arr := ({ints});
  constant SIZES : int_arr := ({sizes});
  constant WS    : int_arr := {widths};
  constant WSC   : int_arr := (1, 2, 7, 8, 31);
  constant WSS   : int_arr := (1, 2, 7, 8, 31, 32);
  constant WSH   : int_arr := (1, 2, 3, 4);
  function ch(c : std_ulogic) return character is begin
    case c is when 'U' => return 'U'; when 'X' => return 'X'; when '0' => return '0'; when '1' => return '1';
      when 'Z' => return 'Z'; when 'W' => return 'W'; when 'L' => return 'L'; when 'H' => return 'H'; when '-' => return '-'; end case;
  end function;
  function elems(v : std_logic_vector) return string is
    variable r : string(1 to v'length);
    variable i : natural := 0;
  begin
    for p in v'range loop i := i + 1; r(i) := ch(v(p)); end loop;
    return r;
  end function;
  function img(v : std_logic_vector) return string is
  begin
    return integer'image(v'left) & "|" & integer'image(v'right) & "|" & boolean'image(v'ascending) & "|" & elems(v);
  end function;
  function rnd(seed, i : integer) return integer is
    variable h : integer;
  begin
    h := (seed * 7919 + i * 104729 + 12345) mod 46337;
    h := (h * h + 17) mod 46337;
    h := (h * 31 + seed) mod 46337;
    return h;
  end function;
  function fill(w, k : natural) return std_logic_vector is
    variable s : std_logic_vector(0 to w - 1);
    variable b : integer;
  begin
    for i in 0 to w - 1 loop
      b := rnd(k + 3 * w + 1, i) mod 2;
      case k mod 16 is
        when 0 => s(i) := '0';
        when 1 => s(i) := '1';
        when 2 => if i mod 2 = 0 then s(i) := '1'; else s(i) := '0'; end if;
        when 3 => if i mod 2 = 0 then s(i) := '0'; else s(i) := '1'; end if;
        when 4 => if i = 0 then s(i) := '1'; else s(i) := '0'; end if;
        when 5 => if i = w - 1 then s(i) := '1'; else s(i) := '0'; end if;
        when 6 | 7 => if b = 1 then s(i) := '1'; else s(i) := '0'; end if;
        when 8 => if i = (w / 2) then s(i) := 'X'; elsif b = 1 then s(i) := '1'; else s(i) := '0'; end if;
        when 9 => if i = (w - 1) then s(i) := 'U'; elsif b = 1 then s(i) := '1'; else s(i) := '0'; end if;
        when 10 => if i = 0 then s(i) := 'Z'; elsif b = 1 then s(i) := '1'; else s(i) := '0'; end if;
        when 11 => if i = (w / 3) then s(i) := 'W'; elsif b = 1 then s(i) := '1'; else s(i) := '0'; end if;
        when 12 => if i = (w - 1) then s(i) := '-'; elsif b = 1 then s(i) := '1'; else s(i) := '0'; end if;
        when 13 => if b = 1 then s(i) := 'H'; else s(i) := 'L'; end if;
        when 14 => s(i) := 'X';
        when others => if i = 0 then s(i) := 'H'; elsif b = 1 then s(i) := '1'; else s(i) := '0'; end if;
      end case;
    end loop;
    return s;
  end function;
  function mkv(w, k, r : natural) return std_logic_vector is
    variable s : std_logic_vector(0 to w - 1) := fill(w, k);
    variable a : std_logic_vector(w - 1 downto 0);
    variable b : std_logic_vector(w + 2 downto 3);
    variable c : std_logic_vector(3 to w + 2);
  begin
    case r mod 4 is
      when 0 => a := s; return a;
      when 1 => return s;
      when 2 => b := s; return b;
      when others => c := s; return c;
    end case;
  end function;
  function mku(w, k, r : natural) return unsigned is begin return unsigned(mkv(w, k, r)); end function;
  function mks(w, k, r : natural) return signed is begin return signed(mkv(w, k, r)); end function;
  function ul(k : natural) return std_ulogic is
    constant t : std_ulogic_vector(0 to 8) := "UX01ZWLH-";
  begin return t(k); end function;
"""

def operand_expr(t, w, k, r, i, u):
    return {"UNSIGNED": f"mku({w},{k},{r})", "SIGNED": f"mks({w},{k},{r})", "STD_LOGIC_VECTOR": f"mkv({w},{k},{r})",
            "INTEGER": f"INTS({i})", "STD_ULOGIC": f"ul({u})"}[t]

def result_img(expr, ret):
    if ret in ("UNSIGNED",): return f"img(std_logic_vector(unsigned'({expr})))"
    if ret in ("SIGNED",): return f"img(std_logic_vector(signed'({expr})))"
    if ret == "STD_LOGIC_VECTOR": return f"img(std_logic_vector'({expr}))"
    if ret == "BOOLEAN": return f"boolean'image({expr})"
    return f"integer'image({expr})"

def gen_sig(idx, op, args, ret, ints="INTS", nints=len(INTS), wsa="WS", wsb="WS"):
    name = f'"{op}"' if not op[0].isalpha() else op
    vec = ("UNSIGNED", "SIGNED", "STD_LOGIC_VECTOR")
    tag = f"{op}[{','.join(args)}->{ret}]"
    isv = [a in vec for a in args]
    body = []
    def call(ex):
        if len(args) == 1 and op[0] != "C" or (len(args) == 1 and False):
            pass
        if op[0].isalpha():
            c = f"{op}({', '.join(ex)})"
        elif args == ("STD_LOGIC_VECTOR", "STD_LOGIC_VECTOR") and op in ("<","<=",">",">=","=","/="):
            # VHDL-93: the predefined SLV compares make a bare operator ambiguous
            c = f'syna.{CURPKG}."{op}"({ex[0]}, {ex[1]})'
        elif len(ex) == 1:
            c = f'"{op}"({ex[0]})'
        else:
            c = f"{ex[0]} {op} {ex[1]}"
        return c
    def emit(ex, info, indent):
        e = call(ex)
        im = []
        for a, x in zip(args, ex):
            if a in vec: im.append(f"img(std_logic_vector({x}))")
            elif a == "INTEGER": im.append(f"integer'image({x})")
            elif a == "STD_ULOGIC": im.append(f"ch({x})")
            else: im.append(f"integer'image({x})")
        info = ' & " , " & '.join(im)
        body.append(" " * indent + f'pr("{tag} " & {info} & " => " & {result_img(e, ret)});')
    out = []
    ni = nints
    if args == ("SIZE",):
        raise ValueError
    a0, a1 = args[0], (args[1] if len(args) > 1 else None)
    L = out.append
    L(f"    -- {tag}")
    if len(args) == 1:
        if a0 in vec:
            L("    for wi in WS'range loop for j in 0 to 15 loop")
            ex = [operand_expr(a0, "WS(wi)", "j", "j+wi", 0, 0)]
            emit(ex, 'integer\'image(WS(wi)) & "," & integer\'image(j)', 6); out += body
            L("    end loop; end loop;")
        elif a0 == "INTEGER":
            L(f"    for i in {ints}'range loop")
            emit([f"{ints}(i)"], f"integer'image({ints}(i))", 6); out += body; L("    end loop;")
        else:
            L("    for u in 0 to 8 loop")
            emit(["ul(u)"], "integer'image(u)", 6); out += body; L("    end loop;")
    elif a1 == "SIZE":
        if a0 in vec:
            L("    for wi in WS'range loop for j in 0 to 15 loop for s in SIZES'range loop")
            emit([operand_expr(a0, "WS(wi)", "j", "j+wi", 0, 0), "SIZES(s)"],
                 'integer\'image(WS(wi)) & "," & integer\'image(j) & "," & integer\'image(SIZES(s))', 6)
        elif a0 == "INTEGER":
            L(f"    for i in {ints}'range loop for s in SIZES'range loop")
            emit([f"{ints}(i)", "SIZES(s)"], f"integer'image({ints}(i)) & \",\" & integer'image(SIZES(s))", 6)
        else:
            L("    for u in 0 to 8 loop for s in SIZES'range loop")
            emit(["ul(u)", "SIZES(s)"], 'integer\'image(u) & "," & integer\'image(SIZES(s))', 6)
        out += body
        L("    end loop; end loop;" if a0 not in vec else "    end loop; end loop; end loop;")
    elif isv[0] and isv[1]:
        L("    for wa in WS'range loop for wb in WS'range loop for j in 0 to 15 loop")
        L("      kb := (j * 7 + wa * 3 + wb) mod 16;")
        ex = [operand_expr(a0, "WS(wa)", "j", "j+wa", 0, 0), operand_expr(a1, "WS(wb)", "kb", "kb+wb+1", 0, 0)]
        emit(ex, 'integer\'image(WS(wa)) & "," & integer\'image(j) & ";" & integer\'image(WS(wb)) & "," & integer\'image(kb)', 6)
        out += body; L("    end loop; end loop; end loop;")
    else:
        # one vector, one integer or std_ulogic (either order)
        vi = 0 if isv[0] else 1
        other = args[1 - vi]
        L("    for wi in WS'range loop for j in 0 to 15 loop for t in 0 to " + ("2" if other == "INTEGER" else "8") + " loop")
        if other == "INTEGER":
            L(f"      ii := (j * 3 + t * 5 + wi) mod {ni};")
        vx = operand_expr(args[vi], "WS(wi)", "j", "j+wi", 0, 0)
        ox = operand_expr(other, 0, 0, 0, "ii", "t")
        ex = [vx, ox] if vi == 0 else [ox, vx]
        emit(ex, 'integer\'image(WS(wi)) & "," & integer\'image(j) & ";" & integer\'image(' + ("ii" if other == "INTEGER" else "t") + ")", 6)
        out += body; L("    end loop; end loop; end loop;")
    t = "\n".join(out) + "\n"
    t = (t.replace("WS(wa)", wsa + "(wa)").replace("WS(wb)", wsb + "(wb)").replace("WS(wi)", wsa + "(wi)")
          .replace("wa in WS'range", "wa in " + wsa + "'range").replace("wb in WS'range", "wb in " + wsb + "'range")
          .replace("wi in WS'range", "wi in " + wsa + "'range"))
    return t

def build(name, sigs, extra, ints=INTS):
    t = HEAD.format(name=name, extra=extra, ints=", ".join(map(str, ints)), sizes=", ".join(map(str, SIZES)), widths=WIDTHS)
    # ints as constant array; allow edge sets
    t += "begin\n  process\n    variable kb, ii : integer;\n  begin\n"
    for i, (op, a, r) in enumerate(sigs):
        wsa, wsb = "WS", "WS"
        if op == "CONV_INTEGER":
            wsa = "WSS" if (a[0] == "SIGNED" or (a[0] == "STD_LOGIC_VECTOR" and "signed.all" in extra and "unsigned.all" not in extra)) else "WSC"
        if op in ("SHL", "SHR"): wsb = "WSH"
        t += gen_sig(i, op, a, r, nints=len(ints), wsa=wsa, wsb=wsb)
    t += '    pr("DONE");\n    wait;\n  end process;\nend architecture;\n'
    return t

def groups(sigs):
    g = {}
    for s in sigs:
        op = s[0]
        if op in ("+", "-") and len(s[1]) == 2: k = "addsub"
        elif op in ("+", "-", "abs"): k = "unary"
        elif op == "*": k = "mul"
        elif op in ("<","<=",">",">=","=","/="): k = "cmp"
        elif op in ("SHL","SHR"): k = "shift"
        elif op == "CONV_INTEGER": k = "conv_integer"
        elif op.startswith("CONV_"): k = "conv_vec"
        else: k = "ext"
        g.setdefault(k, []).append(s)
    return g

def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    plans = [("arith", arith_sigs(), "")]
    plans.append(("unsigned", slv_sigs(False), "use syna.std_logic_unsigned.all;"))
    plans.append(("signed", slv_sigs(True), "use syna.std_logic_signed.all;"))
    global CURPKG
    for pk, sigs, extra in plans:
        CURPKG = "std_logic_" + pk
        for gk, gs in groups(sigs).items():
            nm = f"tb_{pk}_{gk}"
            # tb for the slv packages additionally keeps arith visible (the common usage)
            ints = INTS
            open(os.path.join(out, nm + ".vhd"), "w").write(build(nm, gs, extra, ints))
main()
