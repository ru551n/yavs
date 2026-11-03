-- pgo: top=big_records_tb std=2008 stop=5 us weight=2
-- Reproduces whole-record signal assignments of large nested records: two-process style stages
-- (v := r; a few fields change; rin <= v), record ports, sub-record outputs, arrays of records,
-- chained in lanes and checked against a variable-based model with rolling checksums.

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package br_pkg is
  type mode_t is (M_IDLE, M_LOAD, M_RUN, M_HOLD, M_FLUSH, M_DONE);
  subtype u32_t is unsigned(31 downto 0);
  subtype x64_t is unsigned(63 downto 0);
  type u32_arr_t is array (natural range <>) of u32_t;
  type x64_arr_t is array (natural range <>) of x64_t;


  -- 12 scalar leaves
  type flags_t is record
    en : std_ulogic;
    vld : std_ulogic;
    rdy : std_ulogic;
    err : std_ulogic;
    ovf : std_ulogic;
    zero : std_ulogic;
    neg : std_ulogic;
    par : std_ulogic;
    lk : std_logic;
    st : std_logic;
    rs : std_logic;
    ex : std_logic;
  end record;

  -- 6 scalar leaves
  type cnt_t is record
    n : unsigned(11 downto 0);
    lim : unsigned(11 downto 0);
    idx : integer range 0 to 255;
    wrap : boolean;
    ph : mode_t;
    tag : std_logic_vector(2 downto 0);
  end record;

  -- 7 scalar leaves
  type leaf3_t is record
    a : std_logic_vector(7 downto 0);
    b : unsigned(5 downto 0);
    c : signed(9 downto 0);
    f : std_ulogic;
    k : boolean;
    m : mode_t;
    i : integer range -8 to 7;
  end record;

  -- 23 scalar leaves
  type mid_t is record
    x : leaf3_t;
    y : leaf3_t;
    c : cnt_t;
    w : std_logic_vector(31 downto 0);
    u : unsigned(15 downto 0);
    e : std_ulogic;
  end record;

  -- 6 scalar leaves
  type slot_t is record
    v : std_ulogic;
    addr : unsigned(11 downto 0);
    data : std_logic_vector(31 downto 0);
    st : mode_t;
    tag : integer range 0 to 15;
    dirty : boolean;
  end record;
  type slot_vec_t is array (natural range <>) of slot_t;
  subtype slot_arr4_t is slot_vec_t(0 to 3);
  subtype slot_arr16_t is slot_vec_t(0 to 15);
  type vec_arr_t is array (0 to 7) of std_logic_vector(15 downto 0);

  -- 64 scalar leaves
  type grp_t is record
    m0 : mid_t;
    m1 : mid_t;
    f : flags_t;
    acc : unsigned(63 downto 0);
    sacc : signed(47 downto 0);
    pc : unsigned(15 downto 0);
    n : integer range 0 to 65535;
    sel : std_logic_vector(3 downto 0);
    done : boolean;
  end record;

  -- 118 scalar leaves
  type sm_t is record
    g : grp_t;
    fl : flags_t;
    slots : slot_arr4_t;
    vecs : vec_arr_t;
    cnt : cnt_t;
    tag : std_logic_vector(15 downto 0);
    mode : mode_t;
    vld : std_ulogic;
    last : boolean;
  end record;

  -- 250 scalar leaves
  type lg_t is record
    g0 : grp_t;
    g1 : grp_t;
    fl : flags_t;
    slots : slot_arr16_t;
    cnt : cnt_t;
    vecs : vec_arr_t;
  end record;

  type sm_arr_t is array (natural range <>) of sm_t;
  type lg_arr_t is array (natural range <>) of lg_t;
  type tap_arr_t is array (natural range <>) of slot_arr16_t;

  constant SLOT_INIT : slot_t := (v => '0', addr => (others => '0'), data => (others => '0'),
                                  st => M_IDLE, tag => 0, dirty => false);

  function u32(x : std_ulogic) return u32_t;
  function u32(x : std_ulogic_vector) return u32_t;
  function u32(x : unsigned) return u32_t;
  function u32(x : signed) return u32_t;
  function u32(x : integer) return u32_t;
  function u32(x : boolean) return u32_t;
  function u32(x : mode_t) return u32_t;
  function to_sl(u : unsigned) return std_ulogic;
  function to_bool(u : unsigned) return boolean;
  function to_mode(u : unsigned) return mode_t;
  function to_rng(u : unsigned; lo, hi : integer) return integer;
  function lowb(u : unsigned; n : natural) return natural;
  procedure nxt(variable x : inout x64_t);

  procedure fill(variable x : inout x64_t; variable v : out flags_t);
  procedure fill(variable x : inout x64_t; variable v : out cnt_t);
  procedure fill(variable x : inout x64_t; variable v : out leaf3_t);
  procedure fill(variable x : inout x64_t; variable v : out mid_t);
  procedure fill(variable x : inout x64_t; variable v : out slot_t);
  procedure fill(variable x : inout x64_t; variable v : out grp_t);
  procedure fill(variable x : inout x64_t; variable v : out sm_t);
  procedure fill(variable x : inout x64_t; variable v : out lg_t);
  function init_sm(id : natural) return sm_t;
  function init_lg(id : natural) return lg_t;
  function csum(v : sm_t) return u32_t;
  function csum(v : lg_t) return u32_t;
  procedure nudge(variable x : inout x64_t; variable s : inout sm_t);
end package;

package body br_pkg is

  function u32(x : std_ulogic) return u32_t is
  begin
    if x = '1' then
      return to_unsigned(1, 32);
    end if;
    return to_unsigned(0, 32);
  end function;

  function u32(x : std_ulogic_vector) return u32_t is
  begin
    return resize(unsigned(x), 32);
  end function;

  function u32(x : unsigned) return u32_t is
  begin
    return resize(x, 32);
  end function;

  function u32(x : signed) return u32_t is
  begin
    return unsigned(resize(x, 32));
  end function;

  function u32(x : integer) return u32_t is
  begin
    return unsigned(to_signed(x, 32));
  end function;

  function u32(x : boolean) return u32_t is
  begin
    if x then
      return to_unsigned(1, 32);
    end if;
    return to_unsigned(0, 32);
  end function;

  function u32(x : mode_t) return u32_t is
  begin
    return to_unsigned(mode_t'pos(x), 32);
  end function;

  function to_sl(u : unsigned) return std_ulogic is
  begin
    return u(u'right);
  end function;

  function to_bool(u : unsigned) return boolean is
  begin
    return u(u'right) = '1';
  end function;

  function to_mode(u : unsigned) return mode_t is
  begin
    return mode_t'val(to_integer(u(u'right + 7 downto u'right)) mod 6);
  end function;

  function to_rng(u : unsigned; lo, hi : integer) return integer is
  begin
    return lo + to_integer(u(u'right + 15 downto u'right)) mod (hi - lo + 1);
  end function;

  function lowb(u : unsigned; n : natural) return natural is
  begin
    return to_integer(u(u'right + n - 1 downto u'right));
  end function;

  -- xorshift step
  procedure nxt(variable x : inout x64_t) is
  begin
    x := x xor shift_left(x, 13);
    x := x xor shift_right(x, 7);
    x := x xor shift_left(x, 17);
  end procedure;


  procedure fill(variable x : inout x64_t; variable v : out flags_t) is
  begin
    nxt(x);
    v.en := to_sl(x);
    nxt(x);
    v.vld := to_sl(x);
    nxt(x);
    v.rdy := to_sl(x);
    nxt(x);
    v.err := to_sl(x);
    nxt(x);
    v.ovf := to_sl(x);
    nxt(x);
    v.zero := to_sl(x);
    nxt(x);
    v.neg := to_sl(x);
    nxt(x);
    v.par := to_sl(x);
    nxt(x);
    v.lk := to_sl(x);
    nxt(x);
    v.st := to_sl(x);
    nxt(x);
    v.rs := to_sl(x);
    nxt(x);
    v.ex := to_sl(x);
  end procedure;

  procedure fill(variable x : inout x64_t; variable v : out cnt_t) is
  begin
    nxt(x);
    v.n := resize(x, 12);
    nxt(x);
    v.lim := resize(x, 12);
    nxt(x);
    v.idx := to_rng(x, 0, 255);
    nxt(x);
    v.wrap := to_bool(x);
    nxt(x);
    v.ph := to_mode(x);
    nxt(x);
    v.tag := std_logic_vector(resize(x, 3));
  end procedure;

  procedure fill(variable x : inout x64_t; variable v : out leaf3_t) is
  begin
    nxt(x);
    v.a := std_logic_vector(resize(x, 8));
    nxt(x);
    v.b := resize(x, 6);
    nxt(x);
    v.c := signed(resize(x, 10));
    nxt(x);
    v.f := to_sl(x);
    nxt(x);
    v.k := to_bool(x);
    nxt(x);
    v.m := to_mode(x);
    nxt(x);
    v.i := to_rng(x, -8, 7);
  end procedure;

  procedure fill(variable x : inout x64_t; variable v : out mid_t) is
  begin
    fill(x, v.x);
    fill(x, v.y);
    fill(x, v.c);
    nxt(x);
    v.w := std_logic_vector(resize(x, 32));
    nxt(x);
    v.u := resize(x, 16);
    nxt(x);
    v.e := to_sl(x);
  end procedure;

  procedure fill(variable x : inout x64_t; variable v : out slot_t) is
  begin
    nxt(x);
    v.v := to_sl(x);
    nxt(x);
    v.addr := resize(x, 12);
    nxt(x);
    v.data := std_logic_vector(resize(x, 32));
    nxt(x);
    v.st := to_mode(x);
    nxt(x);
    v.tag := to_rng(x, 0, 15);
    nxt(x);
    v.dirty := to_bool(x);
  end procedure;

  procedure fill(variable x : inout x64_t; variable v : out grp_t) is
  begin
    fill(x, v.m0);
    fill(x, v.m1);
    fill(x, v.f);
    nxt(x);
    v.acc := resize(x, 64);
    nxt(x);
    v.sacc := signed(resize(x, 48));
    nxt(x);
    v.pc := resize(x, 16);
    nxt(x);
    v.n := to_rng(x, 0, 65535);
    nxt(x);
    v.sel := std_logic_vector(resize(x, 4));
    nxt(x);
    v.done := to_bool(x);
  end procedure;

  procedure fill(variable x : inout x64_t; variable v : out sm_t) is
  begin
    fill(x, v.g);
    fill(x, v.fl);
    for k in v.slots'range loop
      fill(x, v.slots(k));
    end loop;
    for k in v.vecs'range loop
      nxt(x);
      v.vecs(k) := std_logic_vector(resize(x, 16));
    end loop;
    fill(x, v.cnt);
    nxt(x);
    v.tag := std_logic_vector(resize(x, 16));
    nxt(x);
    v.mode := to_mode(x);
    nxt(x);
    v.vld := to_sl(x);
    nxt(x);
    v.last := to_bool(x);
  end procedure;

  procedure fill(variable x : inout x64_t; variable v : out lg_t) is
  begin
    fill(x, v.g0);
    fill(x, v.g1);
    fill(x, v.fl);
    for k in v.slots'range loop
      fill(x, v.slots(k));
    end loop;
    fill(x, v.cnt);
    for k in v.vecs'range loop
      nxt(x);
      v.vecs(k) := std_logic_vector(resize(x, 16));
    end loop;
  end procedure;

  function init_sm(id : natural) return sm_t is
    constant K : x64_t := X"9E3779B97F4A7C15";
    variable x : x64_t := to_unsigned(id, 64) + K;
    variable v : sm_t;
  begin
    fill(x, v);
    return v;
  end function;

  function init_lg(id : natural) return lg_t is
    constant K : x64_t := X"9E3779B97F4A7C15";
    variable x : x64_t := to_unsigned(id, 64) + K;
    variable v : lg_t;
  begin
    fill(x, v);
    return v;
  end function;

  function csum(v : flags_t) return u32_t is
    variable a : u32_t := (others => '0');
  begin
    a := rotate_left(a, 3) xor u32(v.en);
    a := rotate_left(a, 3) xor u32(v.rdy);
    a := rotate_left(a, 3) xor u32(v.ovf);
    a := rotate_left(a, 3) xor u32(v.neg);
    a := rotate_left(a, 3) xor u32(v.lk);
    a := rotate_left(a, 3) xor u32(v.rs);
    return a;
  end function;

  function csum(v : cnt_t) return u32_t is
    variable a : u32_t := (others => '0');
  begin
    a := rotate_left(a, 3) xor u32(v.n);
    a := rotate_left(a, 3) xor u32(v.idx);
    a := rotate_left(a, 3) xor u32(v.ph);
    return a;
  end function;

  function csum(v : leaf3_t) return u32_t is
    variable a : u32_t := (others => '0');
  begin
    a := rotate_left(a, 3) xor u32(v.a);
    a := rotate_left(a, 3) xor u32(v.c);
    a := rotate_left(a, 3) xor u32(v.k);
    a := rotate_left(a, 3) xor u32(v.i);
    return a;
  end function;

  function csum(v : mid_t) return u32_t is
    variable a : u32_t := (others => '0');
  begin
    a := rotate_left(a, 5) xor csum(v.x);
    a := rotate_left(a, 5) xor csum(v.y);
    a := rotate_left(a, 5) xor csum(v.c);
    a := rotate_left(a, 3) xor u32(v.w);
    a := rotate_left(a, 3) xor u32(v.e);
    return a;
  end function;

  function csum(v : slot_t) return u32_t is
    variable a : u32_t := (others => '0');
  begin
    a := rotate_left(a, 3) xor u32(v.v);
    a := rotate_left(a, 3) xor u32(v.data);
    a := rotate_left(a, 3) xor u32(v.tag);
    return a;
  end function;

  function csum(v : grp_t) return u32_t is
    variable a : u32_t := (others => '0');
  begin
    a := rotate_left(a, 5) xor csum(v.m0);
    a := rotate_left(a, 5) xor csum(v.m1);
    a := rotate_left(a, 5) xor csum(v.f);
    a := rotate_left(a, 3) xor u32(v.acc);
    a := rotate_left(a, 3) xor u32(v.pc);
    a := rotate_left(a, 3) xor u32(v.sel);
    return a;
  end function;

  function csum(v : sm_t) return u32_t is
    variable a : u32_t := (others => '0');
  begin
    a := rotate_left(a, 5) xor csum(v.g);
    a := rotate_left(a, 5) xor csum(v.fl);
    for k in v.slots'range loop
      a := rotate_left(a, 1) xor csum(v.slots(k));
    end loop;
    for k in v.vecs'range loop
      a := rotate_left(a, 1) xor u32(v.vecs(k));
    end loop;
    a := rotate_left(a, 5) xor csum(v.cnt);
    a := rotate_left(a, 3) xor u32(v.tag);
    a := rotate_left(a, 3) xor u32(v.vld);
    return a;
  end function;

  function csum(v : lg_t) return u32_t is
    variable a : u32_t := (others => '0');
  begin
    a := rotate_left(a, 5) xor csum(v.g0);
    a := rotate_left(a, 5) xor csum(v.g1);
    a := rotate_left(a, 5) xor csum(v.fl);
    for k in v.slots'range loop
      a := rotate_left(a, 1) xor csum(v.slots(k));
    end loop;
    a := rotate_left(a, 5) xor csum(v.cnt);
    for k in v.vecs'range loop
      a := rotate_left(a, 1) xor u32(v.vecs(k));
    end loop;
    return a;
  end function;

  -- Change a handful of fields of the stimulus record.
  procedure nudge(variable x : inout x64_t; variable s : inout sm_t) is
  begin
    nxt(x);
    s.fl.en  := x(0);
    s.fl.vld := x(1);
    s.g.m0.x.a := std_logic_vector(x(15 downto 8));
    s.cnt.n := x(27 downto 16);
    case lowb(x, 3) is
      when 0 => s.g.acc := x;
      when 1 => s.g.m1.w := std_logic_vector(x(63 downto 32));
      when 2 => s.slots(lowb(x(40 downto 32), 2)).data := std_logic_vector(x(63 downto 32));
      when 3 => s.vecs(lowb(x(40 downto 32), 3)) := std_logic_vector(x(31 downto 16));
      when 4 => s.mode := to_mode(x(31 downto 24));
      when 5 => s.g.m0.y.c := signed(x(47 downto 38));
      when others => s.g.done := x(9) = '1';
    end case;
  end procedure;

end package body;


library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.br_pkg.all;

package br_model is
  procedure comb_a(constant r : in sm_t; constant i : in sm_t; constant ID : in natural; variable v, o : inout sm_t);
  procedure comb_b(constant r : in lg_t; constant i : in sm_t; constant ID : in natural; variable v, o : inout lg_t);
  procedure comb_c(constant r : in lg_t; constant i : in lg_t; constant ID : in natural; variable v, o : inout lg_t);
  procedure comb_d(constant r : in sm_t; constant i : in lg_t; constant ID : in natural; variable v, o : inout sm_t);

  type lane_st_t is record
    r1, v1, o1 : sm_t;
    r2, v2, o2 : lg_t;
    r3, v3, o3 : lg_t;
    r4, v4, o4 : lg_t;
    r5, v5, o5 : sm_t;
    r6, v6, o6 : sm_t;
    r7, v7, o7 : lg_t;
    r8, v8, o8 : lg_t;
    tap : slot_arr16_t;
  end record;
  type lane_st_arr_t is array (natural range <>) of lane_st_t;

  function init_lane(lane : natural) return lane_st_t;
  procedure model_step(variable st : inout lane_st_t; constant lane : in natural; constant inp : in sm_t);
  procedure model_edge(variable st : inout lane_st_t; constant lane : in natural; constant rst : in boolean);
end package;

package body br_model is

  -- stage a as a procedure: the same update the entity process does
  procedure comb_a(constant r : in sm_t; constant i : in sm_t; constant ID : in natural; variable v, o : inout sm_t) is
  begin
    v := r;
    v.g.m0.c.ph := to_mode(rotate_left(u32(i.vld), 6) xor u32(r.slots(0).st));
    v.slots(2).data := std_logic_vector(resize(rotate_left(u32(r.fl.zero), 14) xor u32(i.slots(2).addr), 32));
    v.fl.en := to_sl(u32(r.g.m0.c.ph) + u32(i.g.m1.y.b));
    v.g.m0.y.f := to_sl(u32(i.vld) - u32(i.fl.err));
    if lowb(u32(r.g.f.vld), 3) = 3 then
      v.g.m1.x.i := to_rng(u32(r.slots(3).data) + to_unsigned(ID * 16 + 23, 32), -8, 7);
      v.g.m0.c.ph := to_mode(u32(r.cnt.idx) xor u32(i.slots(2).v));
    end if;
    if i.g.m1.x.k or lowb(u32(i.vecs(3)), 3) = 1 then
      v.fl.par := to_sl(u32(r.cnt.ph) + to_unsigned(ID * 27 + 21, 32));
      v.cnt.lim := resize(u32(r.g.m1.e) - u32(i.g.m1.y.b), 12);
    else
      v.g.m0.x.a := std_logic_vector(resize(u32(i.vecs(1)) and u32(i.g.m1.c.n), 8));
    end if;
    if lowb(u32(i.g.m1.c.ph), 2) < 3 then
      v.g.m0.u := resize(u32(r.mode) + u32(r.g.f.neg), 16);
    else
      v.g.f.ex := to_sl(u32(r.cnt.ph) - u32(i.slots(3).dirty));
    end if;
    if u32(r.slots(0).st) /= u32(i.fl.neg) then
      v.slots(0).addr := resize(u32(r.fl.vld) xor u32(r.g.m1.c.wrap), 12);
    end if;
    if i.fl.err = '1' and lowb(u32(r.g.m0.c.wrap), 2) /= 0 then
      v.g.m1.w := std_logic_vector(resize(u32(r.vecs(0)) and u32(i.slots(2).tag), 32));
      v.g.n := to_rng(rotate_left(u32(r.slots(0).v), 27) xor u32(i.g.m0.c.lim), 0, 65535);
      v.g.m0.y.m := to_mode(u32(r.g.f.rdy) + to_unsigned(ID * 19 + 31, 32));
    end if;
    if r.cnt.idx = 255 then v.cnt.idx := 0; else v.cnt.idx := r.cnt.idx + 1; end if;
    if r.g.n = 65535 then v.g.n := 0; else v.g.n := r.g.n + 1; end if;
    v.cnt.n := r.cnt.n + 1;
    if r.cnt.n = r.cnt.lim then v.cnt.wrap := not r.cnt.wrap; end if;
    v.slots(lowb(u32(r.slots(0).data), 2)).data := std_logic_vector(resize(u32(r.slots(1).dirty) and u32(r.g.m0.y.b), 32));
    v.slots(lowb(u32(r.cnt.ph), 2)).v := '1';
    v.slots(lowb(u32(r.g.m1.y.b), 2)).data := std_logic_vector(resize(u32(i.g.m1.c.n) + to_unsigned(ID * 23 + 22, 32), 32));
    v.slots(lowb(u32(i.g.m0.y.i), 2)).v := '1';
    v.vecs(lowb(u32(r.fl.rs), 3)) := std_logic_vector(resize(u32(r.g.sel) + to_unsigned(ID * 10 + 92, 32), 16));
    if i.g.m1.x.f = '1' and lowb(u32(r.slots(1).v), 5) = 14 then v.g := i.g; end if;
    if i.fl.vld = '1' and lowb(u32(r.g.m0.c.idx), 5) = 13 then v.slots := i.slots; end if;
    if i.fl.ovf = '1' and lowb(u32(i.slots(1).st), 5) = 29 then v.vecs := i.vecs; end if;
    o := v;
  end procedure;

  -- stage b as a procedure: the same update the entity process does
  procedure comb_b(constant r : in lg_t; constant i : in sm_t; constant ID : in natural; variable v, o : inout lg_t) is
  begin
    v := r;
    v.slots(1).data := std_logic_vector(resize(u32(r.slots(3).v) xor u32(r.g1.m0.x.b), 32));
    v.g1.f.st := to_sl(u32(r.slots(13).st) + u32(r.fl.ex));
    v.g0.f.lk := to_sl(u32(i.cnt.tag) and u32(r.g0.m1.x.i));
    v.g1.pc := resize(u32(i.cnt.n) - u32(r.g0.f.ovf), 16);
    if i.g.m0.y.k or lowb(u32(r.g0.m0.y.m), 3) = 1 then
      v.g0.m1.c.ph := to_mode(u32(r.g1.m1.e) + u32(r.slots(2).dirty));
    end if;
    if u32(r.g1.f.rs) /= u32(i.vecs(0)) then
      v.g0.n := to_rng(u32(r.g1.m1.x.b) - u32(r.g1.m1.y.k), 0, 65535);
      v.g0.f.par := to_sl(u32(r.g1.m1.c.lim) + u32(r.g1.f.ex));
    end if;
    if u32(r.g0.m1.x.m) /= u32(i.slots(1).addr) then
      v.slots(5).tag := to_rng(u32(r.slots(13).tag) + to_unsigned(ID * 28 + 49, 32), 0, 15);
      v.g0.acc := resize(rotate_left(u32(i.g.m1.c.idx), 24) xor u32(r.g1.m1.x.c), 64);
      v.slots(5).addr := resize(rotate_left(u32(i.g.m0.x.f), 7) xor u32(r.g1.f.st), 12);
    end if;
    if lowb(u32(i.g.m0.y.k), 2) = 2 then
      v.g1.m1.x.m := to_mode(u32(r.g0.m0.c.idx) + u32(r.slots(5).v));
      v.slots(11).st := to_mode(u32(i.g.m0.c.wrap) xor u32(r.g1.m1.c.idx));
      v.g0.f.en := to_sl(u32(i.g.m1.y.f) + to_unsigned(ID * 28 + 88, 32));
    end if;
    if i.slots(0).v = '1' and lowb(u32(i.slots(1).st), 2) /= 0 then
      v.slots(12).tag := to_rng(u32(r.g0.m1.c.ph) and u32(i.slots(2).tag), 0, 15);
      v.slots(6).v := to_sl(u32(r.slots(15).addr) and u32(i.g.m0.x.f));
    else
      v.g0.m0.c.idx := to_rng(u32(r.slots(6).v) + u32(r.g0.sacc), 0, 255);
    end if;
    if r.slots(3).tag = 15 then v.slots(3).tag := 0; else v.slots(3).tag := r.slots(3).tag + 1; end if;
    if r.cnt.idx = 255 then v.cnt.idx := 0; else v.cnt.idx := r.cnt.idx + 1; end if;
    v.cnt.n := r.cnt.n + 1;
    if r.cnt.n = r.cnt.lim then v.cnt.wrap := not r.cnt.wrap; end if;
    v.slots(lowb(u32(r.cnt.tag), 4)).data := std_logic_vector(resize(rotate_left(u32(i.g.pc), 23) xor u32(r.slots(14).tag), 32));
    v.slots(lowb(u32(r.slots(9).v), 4)).v := '1';
    v.slots(lowb(u32(r.slots(8).st), 4)).data := std_logic_vector(resize(rotate_left(u32(r.slots(11).data), 22) xor u32(i.fl.par), 32));
    v.slots(lowb(u32(i.slots(2).st), 4)).v := '1';
    v.vecs(lowb(u32(i.vecs(2)), 3)) := std_logic_vector(resize(u32(r.slots(3).tag) - u32(r.g1.f.en), 16));
    if i.g.f.rdy = '1' and lowb(u32(i.g.m1.c.ph), 5) = 9 then v.g0 := i.g; end if;
    if i.fl.vld = '1' and lowb(u32(r.g0.f.lk), 5) = 24 then v.slots(0 to 3) := i.slots; end if;
    if i.g.m1.y.f = '1' and lowb(u32(i.g.m1.y.a), 5) = 22 then v.g1.m0 := r.g0.m1; end if;
    if i.fl.ex = '1' and lowb(u32(i.g.m1.x.i), 5) = 12 then v.vecs := i.vecs; end if;
    o.g0 := v.g0;
    o.g1 := r.g1;
    o.fl := v.fl;
    o.slots := v.slots;
    o.cnt := v.cnt;
    o.vecs := v.vecs;
  end procedure;

  -- stage c as a procedure: the same update the entity process does
  procedure comb_c(constant r : in lg_t; constant i : in lg_t; constant ID : in natural; variable v, o : inout lg_t) is
  begin
    v := r;
    v.g0.m1.c.n := resize(u32(r.g1.m0.x.b) - u32(i.slots(7).addr), 12);
    v.g0.m1.w := std_logic_vector(resize(u32(r.g1.f.ovf) and u32(r.g1.m1.x.b), 32));
    v.slots(2).st := to_mode(u32(i.slots(9).data) xor u32(i.slots(1).st));
    v.slots(3).v := to_sl(rotate_left(u32(r.g0.m1.c.idx), 2) xor u32(r.g0.f.vld));
    if lowb(u32(r.g0.m1.x.m), 2) < 3 then
      v.slots(8).st := to_mode(u32(r.slots(10).st) and u32(r.slots(10).v));
      v.g0.f.vld := to_sl(u32(r.slots(2).data) - u32(i.slots(8).dirty));
      v.g1.f.err := to_sl(u32(r.g0.m0.y.k) xor u32(i.g1.m0.x.b));
    else
      v.slots(11).addr := resize(u32(r.g1.m1.c.idx) + u32(i.g1.m1.y.a), 12);
    end if;
    if lowb(u32(r.g0.m1.u), 2) < 3 then
      v.slots(14).dirty := to_bool(u32(r.slots(6).data) and u32(i.g1.f.err));
      v.slots(7).v := to_sl(u32(r.g0.m0.c.tag) and u32(i.g1.m0.y.f));
    end if;
    if lowb(u32(i.slots(11).tag), 2) < 3 then
      v.g1.acc := resize(u32(i.g0.m1.y.m) - u32(i.g1.sel), 64);
      v.slots(2).tag := to_rng(u32(r.slots(5).tag) + u32(i.slots(5).v), 0, 15);
      v.g1.m1.y.c := signed(resize(u32(i.g1.m1.y.m) - u32(r.g0.m0.y.k), 10));
    end if;
    if lowb(u32(i.slots(13).data), 2) < 3 then
      v.g0.m1.y.k := to_bool(u32(i.slots(7).tag) xor u32(i.g1.m1.c.tag));
      v.g0.m0.c.ph := to_mode(u32(i.g1.f.ex) and u32(r.g1.m0.y.c));
    end if;
    if i.g0.m1.y.k or lowb(u32(r.g0.m1.y.m), 3) = 1 then
      v.g1.f.rdy := to_sl(u32(r.slots(0).data) - u32(r.slots(10).dirty));
      v.g1.m0.x.k := to_bool(u32(r.g1.m0.y.i) + to_unsigned(ID * 33 + 82, 32));
      v.vecs(0) := std_logic_vector(resize(u32(i.slots(14).tag) and u32(r.slots(9).data), 16));
    end if;
    if r.slots(13).tag = 15 then v.slots(13).tag := 0; else v.slots(13).tag := r.slots(13).tag + 1; end if;
    if r.slots(2).tag = 15 then v.slots(2).tag := 0; else v.slots(2).tag := r.slots(2).tag + 1; end if;
    v.cnt.n := r.cnt.n + 1;
    if r.cnt.n = r.cnt.lim then v.cnt.wrap := not r.cnt.wrap; end if;
    v.slots(lowb(u32(r.g1.m1.y.m), 4)).data := std_logic_vector(resize(u32(i.slots(10).v) xor u32(i.g1.n), 32));
    v.slots(lowb(u32(r.fl.rdy), 4)).v := '1';
    v.slots(lowb(u32(i.g1.m1.c.wrap), 4)).data := std_logic_vector(resize(u32(r.g0.f.ovf) and u32(r.slots(13).v), 32));
    v.slots(lowb(u32(r.g0.m1.x.k), 4)).v := '1';
    v.vecs(lowb(u32(r.g1.m0.x.m), 3)) := std_logic_vector(resize(u32(r.g0.m1.x.a) xor u32(r.slots(6).data), 16));
    if i.fl.ex = '1' and lowb(u32(r.g0.m0.w), 5) = 3 then v.g1 := i.g0; end if;
    if i.slots(2).v = '1' and lowb(u32(i.g0.n), 5) = 30 then v.g0.m1 := i.g1.m0; end if;
    if i.g1.m1.y.f = '1' and lowb(u32(i.slots(2).dirty), 5) = 4 then v.slots(8 to 15) := i.slots(0 to 7); end if;
    if i.slots(1).v = '1' and lowb(u32(r.g1.f.ex), 5) = 16 then v.fl := i.fl; end if;
    o := v;
  end procedure;

  -- stage d as a procedure: the same update the entity process does
  procedure comb_d(constant r : in sm_t; constant i : in lg_t; constant ID : in natural; variable v, o : inout sm_t) is
  begin
    v := r;
    v.slots(2).st := to_mode(rotate_left(u32(r.vecs(6)), 23) xor u32(i.g1.f.en));
    v.g.m0.y.c := signed(resize(u32(i.g0.sel) and u32(r.slots(1).st), 10));
    v.g.f.rdy := to_sl(rotate_left(u32(i.slots(5).st), 4) xor u32(r.slots(3).addr));
    v.g.sel := std_logic_vector(resize(u32(i.g1.m0.x.f) + u32(i.g1.f.err), 4));
    if lowb(u32(i.g0.m0.y.m), 2) < 3 then
      v.g.m1.x.m := to_mode(u32(r.vecs(4)) + u32(i.g1.m0.x.b));
    end if;
    if lowb(u32(i.slots(1).tag), 2) < 3 then
      v.slots(3).addr := resize(rotate_left(u32(r.g.f.vld), 14) xor u32(r.slots(3).v), 12);
    else
      v.vecs(0) := std_logic_vector(resize(u32(r.g.m1.w) - u32(r.fl.rdy), 16));
    end if;
    if u32(r.g.m0.y.k) /= u32(i.g1.m1.y.a) then
      v.fl.rs := to_sl(u32(r.vecs(4)) xor u32(r.fl.rdy));
    end if;
    if i.slots(12).dirty or lowb(u32(i.fl.en), 3) = 1 then
      v.g.m1.y.i := to_rng(u32(r.g.m0.w) xor u32(i.slots(15).dirty), -8, 7);
      v.fl.err := to_sl(u32(r.cnt.ph) xor u32(r.g.m0.c.ph));
      v.g.m1.c.n := resize(u32(i.vecs(4)) + u32(i.g1.m0.y.f), 12);
    else
      v.g.f.par := to_sl(u32(i.fl.vld) and u32(i.g0.f.zero));
    end if;
    if i.slots(15).dirty or lowb(u32(i.slots(6).data), 3) = 1 then
      v.vecs(7) := std_logic_vector(resize(u32(r.cnt.n) - u32(i.g0.f.en), 16));
    end if;
    if r.slots(0).tag = 15 then v.slots(0).tag := 0; else v.slots(0).tag := r.slots(0).tag + 1; end if;
    if r.slots(3).tag = 15 then v.slots(3).tag := 0; else v.slots(3).tag := r.slots(3).tag + 1; end if;
    v.cnt.n := r.cnt.n + 1;
    if r.cnt.n = r.cnt.lim then v.cnt.wrap := not r.cnt.wrap; end if;
    v.slots(lowb(u32(i.g0.done), 2)).data := std_logic_vector(resize(u32(i.g0.f.rdy) and u32(r.cnt.ph), 32));
    v.slots(lowb(u32(i.slots(9).data), 2)).v := '1';
    v.slots(lowb(u32(i.slots(1).st), 2)).data := std_logic_vector(resize(u32(r.g.m1.y.b) + u32(i.vecs(2)), 32));
    v.slots(lowb(u32(r.g.f.st), 2)).v := '1';
    v.vecs(lowb(u32(r.fl.err), 3)) := std_logic_vector(resize(rotate_left(u32(i.slots(9).addr), 9) xor u32(i.slots(14).st), 16));
    if i.g0.f.rdy = '1' and lowb(u32(i.g1.m1.x.f), 5) = 28 then v.g := i.g1; end if;
    if i.g0.m1.e = '1' and lowb(u32(i.slots(4).data), 5) = 18 then v.slots := i.slots(12 to 15); end if;
    if i.g0.m0.y.f = '1' and lowb(u32(r.g.m1.x.m), 5) = 15 then v.cnt := i.cnt; end if;
    if i.g1.f.neg = '1' and lowb(u32(r.fl.ovf), 5) = 14 then v.fl := i.fl; end if;
    o.g := v.g;
    o.fl := v.fl;
    o.slots := v.slots;
    o.vecs := r.vecs;
    o.cnt := v.cnt;
    o.tag := r.tag;
    o.mode := r.mode;
    o.vld := v.vld;
    o.last := v.last;
  end procedure;

  function init_lane(lane : natural) return lane_st_t is
    variable st : lane_st_t;
  begin
    st.r1 := init_sm(lane * 8 + 1);
    st.v1 := st.r1;
    st.o1 := st.r1;
    st.r2 := init_lg(lane * 8 + 2);
    st.v2 := st.r2;
    st.o2 := st.r2;
    st.r3 := init_lg(lane * 8 + 3);
    st.v3 := st.r3;
    st.o3 := st.r3;
    st.r4 := init_lg(lane * 8 + 4);
    st.v4 := st.r4;
    st.o4 := st.r4;
    st.r5 := init_sm(lane * 8 + 5);
    st.v5 := st.r5;
    st.o5 := st.r5;
    st.r6 := init_sm(lane * 8 + 6);
    st.v6 := st.r6;
    st.o6 := st.r6;
    st.r7 := init_lg(lane * 8 + 7);
    st.v7 := st.r7;
    st.o7 := st.r7;
    st.r8 := init_lg(lane * 8 + 8);
    st.v8 := st.r8;
    st.o8 := st.r8;
    st.tap := (others => SLOT_INIT);
    return st;
  end function;

  procedure model_step(variable st : inout lane_st_t; constant lane : in natural; constant inp : in sm_t) is
  begin
    comb_a(st.r1, inp, lane * 8 + 1, st.v1, st.o1);
    comb_b(st.r2, st.o1, lane * 8 + 2, st.v2, st.o2);
    comb_c(st.r3, st.o2, lane * 8 + 3, st.v3, st.o3);
    comb_c(st.r4, st.o3, lane * 8 + 4, st.v4, st.o4);
    comb_d(st.r5, st.o4, lane * 8 + 5, st.v5, st.o5);
    comb_a(st.r6, st.o5, lane * 8 + 6, st.v6, st.o6);
    comb_b(st.r7, st.o6, lane * 8 + 7, st.v7, st.o7);
    comb_c(st.r8, st.o7, lane * 8 + 8, st.v8, st.o8);
  end procedure;

  procedure model_edge(variable st : inout lane_st_t; constant lane : in natural; constant rst : in boolean) is
  begin
    st.tap := st.o8.slots;
    if rst then
      st.r1 := init_sm(lane * 8 + 1);
      st.r2 := init_lg(lane * 8 + 2);
      st.r3 := init_lg(lane * 8 + 3);
      st.r4 := init_lg(lane * 8 + 4);
      st.r5 := init_sm(lane * 8 + 5);
      st.r6 := init_sm(lane * 8 + 6);
      st.r7 := init_lg(lane * 8 + 7);
      st.r8 := init_lg(lane * 8 + 8);
    else
      st.r1 := st.v1;
      st.r2 := st.v2;
      st.r3 := st.v3;
      st.r4 := st.v4;
      st.r5 := st.v5;
      st.r6 := st.v6;
      st.r7 := st.v7;
      st.r8 := st.v8;
    end if;
  end procedure;

end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.br_pkg.all;

entity br_stage_a is
  generic (ID : natural);
  port (clk : in std_ulogic;
        rst : in std_ulogic;
        i   : in sm_t;
        o   : out sm_t := init_sm(0));
end entity;

architecture rtl of br_stage_a is
  constant INIT : sm_t := init_sm(ID);
  signal r, rin : sm_t := INIT;
begin
  comb : process (r, i)
    variable v : sm_t;
  begin
    v := r;
    v.g.m0.c.ph := to_mode(rotate_left(u32(i.vld), 6) xor u32(r.slots(0).st));
    v.slots(2).data := std_logic_vector(resize(rotate_left(u32(r.fl.zero), 14) xor u32(i.slots(2).addr), 32));
    v.fl.en := to_sl(u32(r.g.m0.c.ph) + u32(i.g.m1.y.b));
    v.g.m0.y.f := to_sl(u32(i.vld) - u32(i.fl.err));
    if lowb(u32(r.g.f.vld), 3) = 3 then
      v.g.m1.x.i := to_rng(u32(r.slots(3).data) + to_unsigned(ID * 16 + 23, 32), -8, 7);
      v.g.m0.c.ph := to_mode(u32(r.cnt.idx) xor u32(i.slots(2).v));
    end if;
    if i.g.m1.x.k or lowb(u32(i.vecs(3)), 3) = 1 then
      v.fl.par := to_sl(u32(r.cnt.ph) + to_unsigned(ID * 27 + 21, 32));
      v.cnt.lim := resize(u32(r.g.m1.e) - u32(i.g.m1.y.b), 12);
    else
      v.g.m0.x.a := std_logic_vector(resize(u32(i.vecs(1)) and u32(i.g.m1.c.n), 8));
    end if;
    if lowb(u32(i.g.m1.c.ph), 2) < 3 then
      v.g.m0.u := resize(u32(r.mode) + u32(r.g.f.neg), 16);
    else
      v.g.f.ex := to_sl(u32(r.cnt.ph) - u32(i.slots(3).dirty));
    end if;
    if u32(r.slots(0).st) /= u32(i.fl.neg) then
      v.slots(0).addr := resize(u32(r.fl.vld) xor u32(r.g.m1.c.wrap), 12);
    end if;
    if i.fl.err = '1' and lowb(u32(r.g.m0.c.wrap), 2) /= 0 then
      v.g.m1.w := std_logic_vector(resize(u32(r.vecs(0)) and u32(i.slots(2).tag), 32));
      v.g.n := to_rng(rotate_left(u32(r.slots(0).v), 27) xor u32(i.g.m0.c.lim), 0, 65535);
      v.g.m0.y.m := to_mode(u32(r.g.f.rdy) + to_unsigned(ID * 19 + 31, 32));
    end if;
    if r.cnt.idx = 255 then v.cnt.idx := 0; else v.cnt.idx := r.cnt.idx + 1; end if;
    if r.g.n = 65535 then v.g.n := 0; else v.g.n := r.g.n + 1; end if;
    v.cnt.n := r.cnt.n + 1;
    if r.cnt.n = r.cnt.lim then v.cnt.wrap := not r.cnt.wrap; end if;
    v.slots(lowb(u32(r.slots(0).data), 2)).data := std_logic_vector(resize(u32(r.slots(1).dirty) and u32(r.g.m0.y.b), 32));
    v.slots(lowb(u32(r.cnt.ph), 2)).v := '1';
    v.slots(lowb(u32(r.g.m1.y.b), 2)).data := std_logic_vector(resize(u32(i.g.m1.c.n) + to_unsigned(ID * 23 + 22, 32), 32));
    v.slots(lowb(u32(i.g.m0.y.i), 2)).v := '1';
    v.vecs(lowb(u32(r.fl.rs), 3)) := std_logic_vector(resize(u32(r.g.sel) + to_unsigned(ID * 10 + 92, 32), 16));
    if i.g.m1.x.f = '1' and lowb(u32(r.slots(1).v), 5) = 14 then v.g := i.g; end if;
    if i.fl.vld = '1' and lowb(u32(r.g.m0.c.idx), 5) = 13 then v.slots := i.slots; end if;
    if i.fl.ovf = '1' and lowb(u32(i.slots(1).st), 5) = 29 then v.vecs := i.vecs; end if;
    rin <= v;
    o <= v;
  end process;

  regs : process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        r <= INIT;
      else
        r <= rin;
      end if;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.br_pkg.all;

entity br_stage_b is
  generic (ID : natural);
  port (clk : in std_ulogic;
        rst : in std_ulogic;
        i   : in sm_t;
        o   : out lg_t := init_lg(0));
end entity;

architecture rtl of br_stage_b is
  constant INIT : lg_t := init_lg(ID);
  signal r, rin : lg_t := INIT;
begin
  comb : process (r, i)
    variable v : lg_t;
  begin
    v := r;
    v.slots(1).data := std_logic_vector(resize(u32(r.slots(3).v) xor u32(r.g1.m0.x.b), 32));
    v.g1.f.st := to_sl(u32(r.slots(13).st) + u32(r.fl.ex));
    v.g0.f.lk := to_sl(u32(i.cnt.tag) and u32(r.g0.m1.x.i));
    v.g1.pc := resize(u32(i.cnt.n) - u32(r.g0.f.ovf), 16);
    if i.g.m0.y.k or lowb(u32(r.g0.m0.y.m), 3) = 1 then
      v.g0.m1.c.ph := to_mode(u32(r.g1.m1.e) + u32(r.slots(2).dirty));
    end if;
    if u32(r.g1.f.rs) /= u32(i.vecs(0)) then
      v.g0.n := to_rng(u32(r.g1.m1.x.b) - u32(r.g1.m1.y.k), 0, 65535);
      v.g0.f.par := to_sl(u32(r.g1.m1.c.lim) + u32(r.g1.f.ex));
    end if;
    if u32(r.g0.m1.x.m) /= u32(i.slots(1).addr) then
      v.slots(5).tag := to_rng(u32(r.slots(13).tag) + to_unsigned(ID * 28 + 49, 32), 0, 15);
      v.g0.acc := resize(rotate_left(u32(i.g.m1.c.idx), 24) xor u32(r.g1.m1.x.c), 64);
      v.slots(5).addr := resize(rotate_left(u32(i.g.m0.x.f), 7) xor u32(r.g1.f.st), 12);
    end if;
    if lowb(u32(i.g.m0.y.k), 2) = 2 then
      v.g1.m1.x.m := to_mode(u32(r.g0.m0.c.idx) + u32(r.slots(5).v));
      v.slots(11).st := to_mode(u32(i.g.m0.c.wrap) xor u32(r.g1.m1.c.idx));
      v.g0.f.en := to_sl(u32(i.g.m1.y.f) + to_unsigned(ID * 28 + 88, 32));
    end if;
    if i.slots(0).v = '1' and lowb(u32(i.slots(1).st), 2) /= 0 then
      v.slots(12).tag := to_rng(u32(r.g0.m1.c.ph) and u32(i.slots(2).tag), 0, 15);
      v.slots(6).v := to_sl(u32(r.slots(15).addr) and u32(i.g.m0.x.f));
    else
      v.g0.m0.c.idx := to_rng(u32(r.slots(6).v) + u32(r.g0.sacc), 0, 255);
    end if;
    if r.slots(3).tag = 15 then v.slots(3).tag := 0; else v.slots(3).tag := r.slots(3).tag + 1; end if;
    if r.cnt.idx = 255 then v.cnt.idx := 0; else v.cnt.idx := r.cnt.idx + 1; end if;
    v.cnt.n := r.cnt.n + 1;
    if r.cnt.n = r.cnt.lim then v.cnt.wrap := not r.cnt.wrap; end if;
    v.slots(lowb(u32(r.cnt.tag), 4)).data := std_logic_vector(resize(rotate_left(u32(i.g.pc), 23) xor u32(r.slots(14).tag), 32));
    v.slots(lowb(u32(r.slots(9).v), 4)).v := '1';
    v.slots(lowb(u32(r.slots(8).st), 4)).data := std_logic_vector(resize(rotate_left(u32(r.slots(11).data), 22) xor u32(i.fl.par), 32));
    v.slots(lowb(u32(i.slots(2).st), 4)).v := '1';
    v.vecs(lowb(u32(i.vecs(2)), 3)) := std_logic_vector(resize(u32(r.slots(3).tag) - u32(r.g1.f.en), 16));
    if i.g.f.rdy = '1' and lowb(u32(i.g.m1.c.ph), 5) = 9 then v.g0 := i.g; end if;
    if i.fl.vld = '1' and lowb(u32(r.g0.f.lk), 5) = 24 then v.slots(0 to 3) := i.slots; end if;
    if i.g.m1.y.f = '1' and lowb(u32(i.g.m1.y.a), 5) = 22 then v.g1.m0 := r.g0.m1; end if;
    if i.fl.ex = '1' and lowb(u32(i.g.m1.x.i), 5) = 12 then v.vecs := i.vecs; end if;
    rin <= v;
    o.g0 <= v.g0;
    o.g1 <= r.g1;
    o.fl <= v.fl;
    o.slots <= v.slots;
    o.cnt <= v.cnt;
    o.vecs <= v.vecs;
  end process;

  regs : process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        r <= INIT;
      else
        r <= rin;
      end if;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.br_pkg.all;

entity br_stage_c is
  generic (ID : natural);
  port (clk : in std_ulogic;
        rst : in std_ulogic;
        i   : in lg_t;
        o   : out lg_t := init_lg(0));
end entity;

architecture rtl of br_stage_c is
  constant INIT : lg_t := init_lg(ID);
  signal r, rin : lg_t := INIT;
begin
  comb : process (r, i)
    variable v : lg_t;
  begin
    v := r;
    v.g0.m1.c.n := resize(u32(r.g1.m0.x.b) - u32(i.slots(7).addr), 12);
    v.g0.m1.w := std_logic_vector(resize(u32(r.g1.f.ovf) and u32(r.g1.m1.x.b), 32));
    v.slots(2).st := to_mode(u32(i.slots(9).data) xor u32(i.slots(1).st));
    v.slots(3).v := to_sl(rotate_left(u32(r.g0.m1.c.idx), 2) xor u32(r.g0.f.vld));
    if lowb(u32(r.g0.m1.x.m), 2) < 3 then
      v.slots(8).st := to_mode(u32(r.slots(10).st) and u32(r.slots(10).v));
      v.g0.f.vld := to_sl(u32(r.slots(2).data) - u32(i.slots(8).dirty));
      v.g1.f.err := to_sl(u32(r.g0.m0.y.k) xor u32(i.g1.m0.x.b));
    else
      v.slots(11).addr := resize(u32(r.g1.m1.c.idx) + u32(i.g1.m1.y.a), 12);
    end if;
    if lowb(u32(r.g0.m1.u), 2) < 3 then
      v.slots(14).dirty := to_bool(u32(r.slots(6).data) and u32(i.g1.f.err));
      v.slots(7).v := to_sl(u32(r.g0.m0.c.tag) and u32(i.g1.m0.y.f));
    end if;
    if lowb(u32(i.slots(11).tag), 2) < 3 then
      v.g1.acc := resize(u32(i.g0.m1.y.m) - u32(i.g1.sel), 64);
      v.slots(2).tag := to_rng(u32(r.slots(5).tag) + u32(i.slots(5).v), 0, 15);
      v.g1.m1.y.c := signed(resize(u32(i.g1.m1.y.m) - u32(r.g0.m0.y.k), 10));
    end if;
    if lowb(u32(i.slots(13).data), 2) < 3 then
      v.g0.m1.y.k := to_bool(u32(i.slots(7).tag) xor u32(i.g1.m1.c.tag));
      v.g0.m0.c.ph := to_mode(u32(i.g1.f.ex) and u32(r.g1.m0.y.c));
    end if;
    if i.g0.m1.y.k or lowb(u32(r.g0.m1.y.m), 3) = 1 then
      v.g1.f.rdy := to_sl(u32(r.slots(0).data) - u32(r.slots(10).dirty));
      v.g1.m0.x.k := to_bool(u32(r.g1.m0.y.i) + to_unsigned(ID * 33 + 82, 32));
      v.vecs(0) := std_logic_vector(resize(u32(i.slots(14).tag) and u32(r.slots(9).data), 16));
    end if;
    if r.slots(13).tag = 15 then v.slots(13).tag := 0; else v.slots(13).tag := r.slots(13).tag + 1; end if;
    if r.slots(2).tag = 15 then v.slots(2).tag := 0; else v.slots(2).tag := r.slots(2).tag + 1; end if;
    v.cnt.n := r.cnt.n + 1;
    if r.cnt.n = r.cnt.lim then v.cnt.wrap := not r.cnt.wrap; end if;
    v.slots(lowb(u32(r.g1.m1.y.m), 4)).data := std_logic_vector(resize(u32(i.slots(10).v) xor u32(i.g1.n), 32));
    v.slots(lowb(u32(r.fl.rdy), 4)).v := '1';
    v.slots(lowb(u32(i.g1.m1.c.wrap), 4)).data := std_logic_vector(resize(u32(r.g0.f.ovf) and u32(r.slots(13).v), 32));
    v.slots(lowb(u32(r.g0.m1.x.k), 4)).v := '1';
    v.vecs(lowb(u32(r.g1.m0.x.m), 3)) := std_logic_vector(resize(u32(r.g0.m1.x.a) xor u32(r.slots(6).data), 16));
    if i.fl.ex = '1' and lowb(u32(r.g0.m0.w), 5) = 3 then v.g1 := i.g0; end if;
    if i.slots(2).v = '1' and lowb(u32(i.g0.n), 5) = 30 then v.g0.m1 := i.g1.m0; end if;
    if i.g1.m1.y.f = '1' and lowb(u32(i.slots(2).dirty), 5) = 4 then v.slots(8 to 15) := i.slots(0 to 7); end if;
    if i.slots(1).v = '1' and lowb(u32(r.g1.f.ex), 5) = 16 then v.fl := i.fl; end if;
    rin <= v;
    o <= v;
  end process;

  regs : process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        r <= INIT;
      else
        r <= rin;
      end if;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.br_pkg.all;

entity br_stage_d is
  generic (ID : natural);
  port (clk : in std_ulogic;
        rst : in std_ulogic;
        i   : in lg_t;
        o   : out sm_t := init_sm(0));
end entity;

architecture rtl of br_stage_d is
  constant INIT : sm_t := init_sm(ID);
  signal r, rin : sm_t := INIT;
begin
  comb : process (r, i)
    variable v : sm_t;
  begin
    v := r;
    v.slots(2).st := to_mode(rotate_left(u32(r.vecs(6)), 23) xor u32(i.g1.f.en));
    v.g.m0.y.c := signed(resize(u32(i.g0.sel) and u32(r.slots(1).st), 10));
    v.g.f.rdy := to_sl(rotate_left(u32(i.slots(5).st), 4) xor u32(r.slots(3).addr));
    v.g.sel := std_logic_vector(resize(u32(i.g1.m0.x.f) + u32(i.g1.f.err), 4));
    if lowb(u32(i.g0.m0.y.m), 2) < 3 then
      v.g.m1.x.m := to_mode(u32(r.vecs(4)) + u32(i.g1.m0.x.b));
    end if;
    if lowb(u32(i.slots(1).tag), 2) < 3 then
      v.slots(3).addr := resize(rotate_left(u32(r.g.f.vld), 14) xor u32(r.slots(3).v), 12);
    else
      v.vecs(0) := std_logic_vector(resize(u32(r.g.m1.w) - u32(r.fl.rdy), 16));
    end if;
    if u32(r.g.m0.y.k) /= u32(i.g1.m1.y.a) then
      v.fl.rs := to_sl(u32(r.vecs(4)) xor u32(r.fl.rdy));
    end if;
    if i.slots(12).dirty or lowb(u32(i.fl.en), 3) = 1 then
      v.g.m1.y.i := to_rng(u32(r.g.m0.w) xor u32(i.slots(15).dirty), -8, 7);
      v.fl.err := to_sl(u32(r.cnt.ph) xor u32(r.g.m0.c.ph));
      v.g.m1.c.n := resize(u32(i.vecs(4)) + u32(i.g1.m0.y.f), 12);
    else
      v.g.f.par := to_sl(u32(i.fl.vld) and u32(i.g0.f.zero));
    end if;
    if i.slots(15).dirty or lowb(u32(i.slots(6).data), 3) = 1 then
      v.vecs(7) := std_logic_vector(resize(u32(r.cnt.n) - u32(i.g0.f.en), 16));
    end if;
    if r.slots(0).tag = 15 then v.slots(0).tag := 0; else v.slots(0).tag := r.slots(0).tag + 1; end if;
    if r.slots(3).tag = 15 then v.slots(3).tag := 0; else v.slots(3).tag := r.slots(3).tag + 1; end if;
    v.cnt.n := r.cnt.n + 1;
    if r.cnt.n = r.cnt.lim then v.cnt.wrap := not r.cnt.wrap; end if;
    v.slots(lowb(u32(i.g0.done), 2)).data := std_logic_vector(resize(u32(i.g0.f.rdy) and u32(r.cnt.ph), 32));
    v.slots(lowb(u32(i.slots(9).data), 2)).v := '1';
    v.slots(lowb(u32(i.slots(1).st), 2)).data := std_logic_vector(resize(u32(r.g.m1.y.b) + u32(i.vecs(2)), 32));
    v.slots(lowb(u32(r.g.f.st), 2)).v := '1';
    v.vecs(lowb(u32(r.fl.err), 3)) := std_logic_vector(resize(rotate_left(u32(i.slots(9).addr), 9) xor u32(i.slots(14).st), 16));
    if i.g0.f.rdy = '1' and lowb(u32(i.g1.m1.x.f), 5) = 28 then v.g := i.g1; end if;
    if i.g0.m1.e = '1' and lowb(u32(i.slots(4).data), 5) = 18 then v.slots := i.slots(12 to 15); end if;
    if i.g0.m0.y.f = '1' and lowb(u32(r.g.m1.x.m), 5) = 15 then v.cnt := i.cnt; end if;
    if i.g1.f.neg = '1' and lowb(u32(r.fl.ovf), 5) = 14 then v.fl := i.fl; end if;
    rin <= v;
    o.g <= v.g;
    o.fl <= v.fl;
    o.slots <= v.slots;
    o.vecs <= r.vecs;
    o.cnt <= v.cnt;
    o.tag <= r.tag;
    o.mode <= r.mode;
    o.vld <= v.vld;
    o.last <= v.last;
  end process;

  regs : process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        r <= INIT;
      else
        r <= rin;
      end if;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.br_pkg.all;

entity br_lane is
  generic (LANE : natural);
  port (clk   : in std_ulogic;
        rst   : in std_ulogic;
        i     : in sm_t;
        mid   : out sm_t := init_sm(0);
        o     : out lg_t := init_lg(0);
        tap_o : out slot_arr16_t := (others => SLOT_INIT));
end entity;

architecture rtl of br_lane is
  signal o1 : sm_t := init_sm(0);
  signal o2 : lg_t := init_lg(0);
  signal o3 : lg_t := init_lg(0);
  signal o4 : lg_t := init_lg(0);
  signal o5 : sm_t := init_sm(0);
  signal o6 : sm_t := init_sm(0);
  signal o7 : lg_t := init_lg(0);
  signal o8 : lg_t := init_lg(0);
  signal tap, tap_n : slot_arr16_t := (others => SLOT_INIT);
begin
  s1 : entity work.br_stage_a generic map (ID => LANE * 8 + 1)
    port map (clk => clk, rst => rst, i => i, o => o1);
  s2 : entity work.br_stage_b generic map (ID => LANE * 8 + 2)
    port map (clk => clk, rst => rst, i => o1, o => o2);
  s3 : entity work.br_stage_c generic map (ID => LANE * 8 + 3)
    port map (clk => clk, rst => rst, i => o2, o => o3);
  s4 : entity work.br_stage_c generic map (ID => LANE * 8 + 4)
    port map (clk => clk, rst => rst, i => o3, o => o4);
  s5 : entity work.br_stage_d generic map (ID => LANE * 8 + 5)
    port map (clk => clk, rst => rst, i => o4, o => o5);
  s6 : entity work.br_stage_a generic map (ID => LANE * 8 + 6)
    port map (clk => clk, rst => rst, i => o5, o => o6);
  s7 : entity work.br_stage_b generic map (ID => LANE * 8 + 7)
    port map (clk => clk, rst => rst, i => o6, o => o7);
  s8 : entity work.br_stage_c generic map (ID => LANE * 8 + 8)
    port map (clk => clk, rst => rst, i => o7, o => o8);

  mid <= o5;
  o <= o8;

  -- delayed copy of the slot array, assigned whole
  tap_comb : process (o8)
  begin
    tap_n <= o8.slots;
  end process;

  tap_reg : process (clk)
  begin
    if rising_edge(clk) then
      tap <= tap_n;
    end if;
  end process;
  tap_o <= tap;
end architecture;


library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.br_pkg.all;
use work.br_model.all;

entity big_records_tb is
end entity;

architecture sim of big_records_tb is
  constant LANES  : positive := 4;
  constant CYCLES : natural := 400;
  -- expected rolling checksums per lane (from the unoptimised interpreter run)
  type gold_t is array (0 to 3) of u32_t;
  constant GOLD : gold_t := (X"41E49D1F", X"A086B9CA", X"8E23B1FA", X"F85DAB6C");
  signal clk : std_ulogic := '0';
  signal rst : std_ulogic := '1';
  signal lane_i   : sm_arr_t(0 to LANES - 1) := (others => init_sm(900));
  signal lane_mid : sm_arr_t(0 to LANES - 1) := (others => init_sm(901));
  signal lane_o   : lg_arr_t(0 to LANES - 1) := (others => init_lg(902));
  signal lane_tap : tap_arr_t(0 to LANES - 1) := (others => (others => SLOT_INIT));
begin
  clk <= not clk after 5 ns;

  lanes : for g in 0 to LANES - 1 generate
    dut : entity work.br_lane
      generic map (LANE => g)
      port map (clk => clk, rst => rst, i => lane_i(g), mid => lane_mid(g),
                o => lane_o(g), tap_o => lane_tap(g));
  end generate;

  stim : process
    variable cur   : sm_arr_t(0 to LANES - 1);
    variable x     : x64_arr_t(0 to LANES - 1);
    variable st    : lane_st_arr_t(0 to LANES - 1);
    variable acc_d : u32_arr_t(0 to LANES - 1) := (others => (others => '0'));
    variable acc_m : u32_arr_t(0 to LANES - 1) := (others => (others => '0'));
    variable prev  : lg_t;
    variable moves : natural := 0;
    variable rst_seen : boolean := true;
  begin
    for g in 0 to LANES - 1 loop
      cur(g) := init_sm(900);
      x(g) := to_unsigned(1000 + 77 * g, 64);
      st(g) := init_lane(g);
      model_step(st(g), g, cur(g));
    end loop;
    prev := lane_o(0);

    for cyc in 0 to CYCLES - 1 loop
      wait until rising_edge(clk);
      for g in 0 to LANES - 1 loop
        model_edge(st(g), g, rst_seen);
      end loop;
      wait for 1 ns;
      for g in 0 to LANES - 1 loop
        nudge(x(g), cur(g));
        -- now and then many fields change at once
        if (cyc + 5 * g) mod 37 = 0 then
          fill(x(g), cur(g));
        end if;
        model_step(st(g), g, cur(g));
      end loop;
      lane_i <= cur;
      rst_seen := cyc < 2;
      if cyc < 2 then
        rst <= '1';
      else
        rst <= '0';
      end if;

      wait until falling_edge(clk);
      for g in 0 to LANES - 1 loop
        assert lane_o(g) = st(g).o8 report "lane output mismatch, lane " & integer'image(g) & " cycle " & integer'image(cyc) severity failure;
        assert lane_mid(g) = st(g).o5 report "lane mid mismatch, lane " & integer'image(g) & " cycle " & integer'image(cyc) severity failure;
        assert lane_tap(g) = st(g).tap report "lane tap mismatch, lane " & integer'image(g) & " cycle " & integer'image(cyc) severity failure;
        acc_d(g) := rotate_left(acc_d(g), 1) xor csum(lane_o(g)) xor csum(lane_mid(g));
        acc_m(g) := rotate_left(acc_m(g), 1) xor csum(st(g).o8) xor csum(st(g).o5);
        assert acc_d(g) = acc_m(g) report "checksum mismatch, lane " & integer'image(g) severity failure;
      end loop;
      if lane_o(0) /= prev then
        moves := moves + 1;
      end if;
      prev := lane_o(0);
    end loop;

    assert moves > CYCLES / 2 report "too little activity" severity failure;
    for g in 0 to LANES - 1 loop
      assert acc_d(g) = GOLD(g) report "golden checksum mismatch, lane " & integer'image(g) severity failure;
    end loop;
    report "PASS" severity note;
    wait;
  end process;
end architecture;
