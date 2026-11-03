-- pgo: top=port_outputs_tb std=2008 stop=41us
-- Combinational processes that end every activation by copying a record
-- variable into many separate scalar and vector output ports, almost always
-- with unchanged values. Three-level hierarchy built by generate, with
-- resolved two-driver signals and inout ports. A rolling checksum of all
-- outputs is checked against a model.

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package po_pkg is
  constant NM : positive := 6;
  constant NL : positive := 4;
  constant NT : positive := NM * NL;
  constant NSETS : positive := 4;

  subtype byte_int is integer range 0 to 255;
  subtype sbyte_int is integer range -128 to 127;
  subtype word_nat is natural range 0 to 1023;
  type state_t is (st_idle, st_load, st_run, st_hold, st_done);
  type mode_t is (m_a, m_b, m_c);

  type stim_t is record
    a    : unsigned(15 downto 0);
    b    : std_logic_vector(7 downto 0);
    sel  : integer range 0 to 15;
    st   : state_t;
    mode : mode_t;
    dir  : std_logic;
    run  : std_logic;
  end record;
  type stim_arr is array (natural range <>) of stim_t;

  type rec_a is record
    s1  : std_logic;
    s2  : std_ulogic;
    b   : boolean;
    i   : byte_int;
    v2  : std_logic_vector(1 downto 0);
    v5  : std_logic_vector(4 downto 0);
    v8  : std_logic_vector(7 downto 0);
    u3  : unsigned(2 downto 0);
    u12 : unsigned(11 downto 0);
    u16 : unsigned(15 downto 0);
    e   : state_t;
    v33 : std_logic_vector(32 downto 0);
  end record;
  type rec_b is record
    s   : std_logic;
    b   : boolean;
    i   : sbyte_int;
    e   : mode_t;
    v7  : std_logic_vector(6 downto 0);
    v16 : std_logic_vector(15 downto 0);
    u24 : unsigned(23 downto 0);
    u40 : unsigned(39 downto 0);
    v64 : std_logic_vector(63 downto 0);
    u64 : unsigned(63 downto 0);
  end record;
  type rec_c is record
    s   : std_logic;
    su  : std_ulogic;
    b   : boolean;
    i   : word_nat;
    e   : state_t;
    v2  : std_logic_vector(1 downto 0);
    v3  : std_logic_vector(2 downto 0);
    v9  : std_logic_vector(8 downto 0);
    v13 : std_logic_vector(12 downto 0);
    v32 : std_logic_vector(31 downto 0);
    u2  : unsigned(1 downto 0);
    u5  : unsigned(4 downto 0);
    u9  : unsigned(8 downto 0);
    u31 : unsigned(30 downto 0);
    u48 : unsigned(47 downto 0);
    u57 : unsigned(56 downto 0);
  end record;
  type rec_a_arr is array (natural range <>) of rec_a;
  type rec_b_arr is array (natural range <>) of rec_b;
  type rec_c_arr is array (natural range <>) of rec_c;
  type slv8_arr is array (natural range <>) of std_logic_vector(7 downto 0);
  type u32_arr is array (natural range <>) of unsigned(31 downto 0);

  function mk_w (a : unsigned(15 downto 0); b : std_logic_vector(7 downto 0);
                 sel : integer range 0 to 15; st : state_t; mode : mode_t;
                 id : natural) return unsigned;
  function mk_a (w : unsigned(63 downto 0)) return rec_a;
  function mk_b (w : unsigned(63 downto 0)) return rec_b;
  function mk_c (w : unsigned(63 downto 0)) return rec_c;
  function pack (r : rec_a) return std_logic_vector;
  function pack (r : rec_b) return std_logic_vector;
  function pack (r : rec_c) return std_logic_vector;
  function fold (acc : unsigned(31 downto 0); p : std_logic_vector) return unsigned;
  function far (i : natural; b : std_logic_vector(7 downto 0)) return std_logic_vector;
end package po_pkg;

package body po_pkg is
  type lut4_t is array (0 to 3) of std_ulogic;
  constant s2_lut : lut4_t := ('0', '1', 'L', 'H');
  constant kc     : unsigned(63 downto 0) := x"9E3779B97F4A7C15";

  function sl (b : boolean) return std_logic is
  begin
    if b then return '1'; else return '0'; end if;
  end function sl;

  function mk_w (a : unsigned(15 downto 0); b : std_logic_vector(7 downto 0);
                 sel : integer range 0 to 15; st : state_t; mode : mode_t;
                 id : natural) return unsigned is
    variable t : unsigned(34 downto 0);
    variable w : unsigned(63 downto 0);
    variable k : unsigned(63 downto 0);
  begin
    t := a & unsigned(b) & to_unsigned(sel, 4) & to_unsigned(state_t'pos(st), 3)
         & to_unsigned(mode_t'pos(mode), 2) & "00";
    w := resize(t, 64);
    k := rotate_left(kc, id mod 64);
    w := w xor shift_left(w, 13);
    w := w xor shift_right(w, 7);
    w := rotate_left(w, id mod 61) + k;
    w := w xor rotate_right(w, 29);
    return w;
  end function mk_w;

  function mk_a (w : unsigned(63 downto 0)) return rec_a is
    variable r : rec_a;
  begin
    r.s1  := w(0);
    r.s2  := s2_lut(to_integer(w(3 downto 2)));
    r.b   := w(4) = '1';
    r.i   := to_integer(w(12 downto 5));
    r.v2  := std_logic_vector(w(14 downto 13));
    r.v5  := std_logic_vector(w(19 downto 15));
    r.v8  := std_logic_vector(w(27 downto 20));
    r.u3  := w(30 downto 28);
    r.u12 := w(42 downto 31);
    r.u16 := w(58 downto 43);
    r.e   := state_t'val(to_integer(w(61 downto 59)) mod 5);
    r.v33 := std_logic_vector(w(32 downto 0));
    return r;
  end function mk_a;

  function mk_b (w : unsigned(63 downto 0)) return rec_b is
    variable r : rec_b;
    variable t : unsigned(63 downto 0);
  begin
    r.s   := w(63);
    r.b   := w(62) = '1';
    r.i   := to_integer(signed(w(7 downto 0)));
    r.e   := mode_t'val(to_integer(w(9 downto 8)) mod 3);
    r.v7  := std_logic_vector(w(16 downto 10));
    r.v16 := std_logic_vector(w(32 downto 17));
    r.u24 := w(56 downto 33);
    t     := rotate_left(w, 3);
    r.u40 := t(39 downto 0);
    r.v64 := std_logic_vector(w xor rotate_right(w, 9));
    r.u64 := shift_left(w, 1);
    return r;
  end function mk_b;

  function mk_c (w : unsigned(63 downto 0)) return rec_c is
    variable r : rec_c;
    variable t : unsigned(63 downto 0);
  begin
    r.s   := w(5);
    r.su  := w(6);
    r.b   := w(7) = '0';
    r.i   := to_integer(w(17 downto 8));
    r.e   := state_t'val(to_integer(w(20 downto 18)) mod 5);
    r.v2  := std_logic_vector(w(22 downto 21));
    r.v3  := std_logic_vector(w(25 downto 23));
    r.v9  := std_logic_vector(w(34 downto 26));
    r.v13 := std_logic_vector(w(47 downto 35));
    r.v32 := std_logic_vector(w(31 downto 0) xor w(63 downto 32));
    r.u2  := w(49 downto 48);
    r.u5  := w(54 downto 50);
    r.u9  := w(63 downto 55);
    r.u31 := w(62 downto 32);
    t     := rotate_left(w, 11);
    r.u48 := t(47 downto 0);
    t     := w xor shift_right(w, 5);
    r.u57 := t(56 downto 0);
    return r;
  end function mk_c;

  function pack (r : rec_a) return std_logic_vector is
  begin
    return std_logic_vector'(r.s1 & std_logic(r.s2) & sl(r.b)
      & std_logic_vector(to_unsigned(r.i, 8)) & r.v2 & r.v5 & r.v8
      & std_logic_vector(r.u3) & std_logic_vector(r.u12) & std_logic_vector(r.u16)
      & std_logic_vector(to_unsigned(state_t'pos(r.e), 3)) & r.v33);
  end function pack;

  function pack (r : rec_b) return std_logic_vector is
  begin
    return std_logic_vector'(r.s & sl(r.b)
      & std_logic_vector(to_signed(r.i, 8))
      & std_logic_vector(to_unsigned(mode_t'pos(r.e), 2))
      & r.v7 & r.v16 & std_logic_vector(r.u24) & std_logic_vector(r.u40)
      & r.v64 & std_logic_vector(r.u64));
  end function pack;

  function pack (r : rec_c) return std_logic_vector is
  begin
    return std_logic_vector'(r.s & std_logic(r.su) & sl(r.b)
      & std_logic_vector(to_unsigned(r.i, 10))
      & std_logic_vector(to_unsigned(state_t'pos(r.e), 3))
      & r.v2 & r.v3 & r.v9 & r.v13 & r.v32
      & std_logic_vector(r.u2) & std_logic_vector(r.u5) & std_logic_vector(r.u9)
      & std_logic_vector(r.u31) & std_logic_vector(r.u48) & std_logic_vector(r.u57));
  end function pack;

  function fold (acc : unsigned(31 downto 0); p : std_logic_vector) return unsigned is
    variable r  : unsigned(31 downto 0) := acc;
    variable q  : unsigned(p'length - 1 downto 0) := unsigned(to_x01(p));
    variable lo : natural := 0;
  begin
    while lo < q'length loop
      r  := rotate_left(r, 5) xor resize(shift_right(q, lo), 32);
      lo := lo + 32;
    end loop;
    return r + 1;
  end function fold;

  function far (i : natural; b : std_logic_vector(7 downto 0)) return std_logic_vector is
  begin
    return b xor std_logic_vector(to_unsigned(i, 8));
  end function far;
end package body po_pkg;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.po_pkg.all;

entity po_leaf is
  generic (id : natural);
  port (
    clk     : in  std_logic;
    a_in    : in  unsigned(15 downto 0);
    b_in    : in  std_logic_vector(7 downto 0);
    sel_in  : in  integer range 0 to 15;
    st_in   : in  state_t;
    mode_in : in  mode_t;
    dir_in  : in  std_logic;
    a_s1    : out std_logic;
    a_s2    : out std_ulogic;
    a_b     : out boolean;
    a_i     : out byte_int;
    a_v2    : out std_logic_vector(1 downto 0);
    a_v5    : out std_logic_vector(4 downto 0);
    a_v8    : out std_logic_vector(7 downto 0);
    a_u3    : out unsigned(2 downto 0);
    a_u12   : out unsigned(11 downto 0);
    a_u16   : out unsigned(15 downto 0);
    a_e     : out state_t;
    a_v33   : out std_logic_vector(32 downto 0);
    b_s     : out std_logic;
    b_b     : out boolean;
    b_i     : out sbyte_int;
    b_e     : out mode_t;
    b_v7    : out std_logic_vector(6 downto 0);
    b_v16   : out std_logic_vector(15 downto 0);
    b_u24   : out unsigned(23 downto 0);
    b_u40   : out unsigned(39 downto 0);
    b_v64   : out std_logic_vector(63 downto 0);
    b_u64   : out unsigned(63 downto 0);
    c_s     : out std_logic;
    c_su    : out std_ulogic;
    c_b     : out boolean;
    c_i     : out word_nat;
    c_e     : out state_t;
    c_v2    : out std_logic_vector(1 downto 0);
    c_v3    : out std_logic_vector(2 downto 0);
    c_v9    : out std_logic_vector(8 downto 0);
    c_v13   : out std_logic_vector(12 downto 0);
    c_v32   : out std_logic_vector(31 downto 0);
    c_u2    : out unsigned(1 downto 0);
    c_u5    : out unsigned(4 downto 0);
    c_u9    : out unsigned(8 downto 0);
    c_u31   : out unsigned(30 downto 0);
    c_u48   : out unsigned(47 downto 0);
    c_u57   : out unsigned(56 downto 0);
    d_bus   : out std_logic;
    d_seen  : out std_logic_vector(7 downto 0);
    io      : inout std_logic_vector(7 downto 0)
  );
end entity po_leaf;

architecture rtl of po_leaf is
  signal ra : rec_a := mk_a((others => '0'));
  signal rb : rec_b := mk_b((others => '0'));
  signal rc : rec_c := mk_c((others => '0'));
begin
  -- Registers: recomputed every edge, but values change only with the inputs.
  reg : process (clk)
    variable w : unsigned(63 downto 0);
  begin
    if rising_edge(clk) then
      w  := mk_w(a_in, b_in, sel_in, st_in, mode_in, id);
      ra <= mk_a(w);
      rb <= mk_b(w);
      rc <= mk_c(w);
    end if;
  end process reg;

  pa : process (all)
    variable v : rec_a;
  begin
    v := ra;
    if is_x(clk) then
      v.s1 := 'X';
    end if;
    a_s1  <= v.s1;
    a_s2  <= v.s2;
    a_b   <= v.b;
    a_i   <= v.i;
    a_v2  <= v.v2;
    a_v5  <= v.v5;
    a_v8  <= v.v8;
    a_u3  <= v.u3;
    a_u12 <= v.u12;
    a_u16 <= v.u16;
    a_e   <= v.e;
    a_v33 <= v.v33;
  end process pa;

  pb : process (clk, rb)
    variable v : rec_b;
  begin
    v := rb;
    if is_x(clk) then
      v.s := 'X';
    end if;
    b_s   <= v.s;
    b_b   <= v.b;
    b_i   <= v.i;
    b_e   <= v.e;
    b_v7  <= v.v7;
    b_v16 <= v.v16;
    b_u24 <= v.u24;
    b_u40 <= v.u40;
    b_v64 <= v.v64;
    b_u64 <= v.u64;
  end process pb;

  pc : process (all)
    variable v : rec_c;
  begin
    v := rc;
    if is_x(clk) then
      v.s := 'X';
    end if;
    c_s   <= v.s;
    c_su  <= v.su;
    c_b   <= v.b;
    c_i   <= v.i;
    c_e   <= v.e;
    c_v2  <= v.v2;
    c_v3  <= v.v3;
    c_v9  <= v.v9;
    c_v13 <= v.v13;
    c_v32 <= v.v32;
    c_u2  <= v.u2;
    c_u5  <= v.u5;
    c_u9  <= v.u9;
    c_u31 <= v.u31;
    c_u48 <= v.u48;
    c_u57 <= v.u57;
  end process pc;

  -- Two drivers on one resolved output, and a shared inout bus.
  drv_hi : process (all)
  begin
    if dir_in = '1' then
      d_bus <= ra.s1;
    else
      d_bus <= 'Z';
    end if;
  end process drv_hi;

  drv_lo : process (all)
  begin
    if dir_in = '0' then
      d_bus <= std_logic(ra.s2);
    else
      d_bus <= 'Z';
    end if;
  end process drv_lo;

  io     <= ra.v8 when dir_in = '1' else (others => 'Z');
  d_seen <= io;
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.po_pkg.all;

entity po_mid is
  generic (base : natural);
  port (
    clk   : in    std_logic;
    stim  : in    stim_t;
    oa    : out   rec_a_arr(0 to NL - 1);
    dig   : out   unsigned(31 downto 0);
    od    : out   std_logic_vector(0 to NL - 1);
    oseen : out   slv8_arr(0 to NL - 1);
    io    : inout slv8_arr(0 to NL - 1)
  );
end entity po_mid;

architecture rtl of po_mid is
  signal sb : rec_b_arr(0 to NL - 1);
  signal sc : rec_c_arr(0 to NL - 1);
begin
  leaves : for l in 0 to NL - 1 generate
    u : entity work.po_leaf
      generic map (id => base + l)
      port map (
        clk => clk, a_in => stim.a, b_in => stim.b, sel_in => stim.sel,
        st_in => stim.st, mode_in => stim.mode, dir_in => stim.dir,
        a_s1 => oa(l).s1, a_s2 => oa(l).s2, a_b => oa(l).b, a_i => oa(l).i,
        a_v2 => oa(l).v2, a_v5 => oa(l).v5, a_v8 => oa(l).v8, a_u3 => oa(l).u3,
        a_u12 => oa(l).u12, a_u16 => oa(l).u16, a_e => oa(l).e, a_v33 => oa(l).v33,
        b_s => sb(l).s, b_b => sb(l).b, b_i => sb(l).i, b_e => sb(l).e,
        b_v7 => sb(l).v7, b_v16 => sb(l).v16, b_u24 => sb(l).u24,
        b_u40 => sb(l).u40, b_v64 => sb(l).v64, b_u64 => sb(l).u64,
        c_s => sc(l).s, c_su => sc(l).su, c_b => sc(l).b, c_i => sc(l).i,
        c_e => sc(l).e, c_v2 => sc(l).v2, c_v3 => sc(l).v3, c_v9 => sc(l).v9,
        c_v13 => sc(l).v13, c_v32 => sc(l).v32, c_u2 => sc(l).u2,
        c_u5 => sc(l).u5, c_u9 => sc(l).u9, c_u31 => sc(l).u31,
        c_u48 => sc(l).u48, c_u57 => sc(l).u57,
        d_bus => od(l), d_seen => oseen(l), io => io(l));
  end generate leaves;

  -- Digest of the leaf outputs that stay inside this level as signals.
  digest : process (clk)
    variable dg : unsigned(31 downto 0) := (others => '0');
  begin
    if rising_edge(clk) then
      if stim.run = '1' then
        for l in 0 to NL - 1 loop
          dg := fold(fold(dg, pack(sb(l))), pack(sc(l)));
        end loop;
      end if;
      dig <= dg;
    end if;
  end process digest;
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.po_pkg.all;

entity po_top is
  port (
    clk   : in    std_logic;
    stim  : in    stim_arr(0 to NSETS - 1);
    oa    : out   rec_a_arr(0 to NT - 1);
    dig   : out   u32_arr(0 to NM - 1);
    od    : out   std_logic_vector(0 to NT - 1);
    oseen : out   slv8_arr(0 to NT - 1);
    io    : inout slv8_arr(0 to NT - 1)
  );
end entity po_top;

architecture rtl of po_top is
begin
  mids : for m in 0 to NM - 1 generate
    u : entity work.po_mid
      generic map (base => m * NL)
      port map (
        clk => clk, stim => stim(m mod NSETS),
        oa => oa(m * NL to m * NL + NL - 1), dig => dig(m),
        od => od(m * NL to m * NL + NL - 1),
        oseen => oseen(m * NL to m * NL + NL - 1),
        io => io(m * NL to m * NL + NL - 1));
  end generate mids;
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.po_pkg.all;

entity port_outputs_tb is
end entity port_outputs_tb;

architecture sim of port_outputs_tb is
  constant n_cyc : positive := 4000;
  type per_t is array (0 to NSETS - 1) of positive;
  constant per : per_t := (23, 31, 47, 59);

  signal clk    : std_logic := '0';
  signal done   : boolean := false;
  signal stim   : stim_arr(0 to NSETS - 1) := (others => (
    a => x"1234", b => x"56", sel => 3, st => st_idle, mode => m_a,
    dir => '1', run => '0'));
  signal oa     : rec_a_arr(0 to NT - 1);
  signal dig    : u32_arr(0 to NM - 1);
  signal od     : std_logic_vector(0 to NT - 1);
  signal oseen  : slv8_arr(0 to NT - 1);
  signal io_bus : slv8_arr(0 to NT - 1);
begin
  dut : entity work.po_top
    port map (clk => clk, stim => stim, oa => oa, dig => dig, od => od,
              oseen => oseen, io => io_bus);

  clkgen : process
  begin
    while not done loop
      clk <= '0';
      wait for 5 ns;
      clk <= '1';
      wait for 5 ns;
    end loop;
    wait;
  end process clkgen;

  -- Far end of every inout bus: drives it when the leaf does not.
  far_end : for i in 0 to NT - 1 generate
    io_bus(i) <= far(i, stim((i / NL) mod NSETS).b) when stim((i / NL) mod NSETS).dir = '0'
                 else (others => 'Z');
  end generate far_end;

  -- Stimulus: each set changes every few dozen cycles, from an LFSR.
  stimulus : process
    variable lf  : unsigned(31 downto 0) := x"ACE11357";
    variable cyc : natural := 0;
    variable s   : stim_t;
  begin
    while cyc < n_cyc + 8 loop
      wait until falling_edge(clk);
      cyc := cyc + 1;
      for k in 0 to NSETS - 1 loop
        s := stim(k);
        if cyc mod per(k) = 0 then
          for r in 1 to 2 loop
            lf := lf xor shift_left(lf, 13);
            lf := lf xor shift_right(lf, 17);
            lf := lf xor shift_left(lf, 5);
          end loop;
          s.a    := lf(15 downto 0);
          s.b    := std_logic_vector(lf(23 downto 16));
          s.sel  := to_integer(lf(27 downto 24));
          s.st   := state_t'val(to_integer(lf(30 downto 28)) mod 5);
          s.mode := mode_t'val(to_integer(lf(9 downto 8)) mod 3);
          if lf(31) = '1' then
            s.dir := not s.dir;
          end if;
        end if;
        if cyc >= 4 then
          s.run := '1';
        end if;
        stim(k) <= s;
      end loop;
    end loop;
    wait;
  end process stimulus;

  -- Model and rolling checksum, sampled at every rising edge.
  check : process
    variable cyc   : natural := 0;
    variable ea    : rec_a_arr(0 to NT - 1);
    variable eb    : rec_b_arr(0 to NT - 1);
    variable ec    : rec_c_arr(0 to NT - 1);
    variable pst   : stim_arr(0 to NSETS - 1);
    variable mdg   : u32_arr(0 to NM - 1) := (others => (others => '0'));
    variable acc_d : unsigned(31 downto 0) := (others => '0');
    variable acc_m : unsigned(31 downto 0) := (others => '0');
    variable s     : natural;
    variable w     : unsigned(63 downto 0);
    variable eio   : std_logic_vector(7 downto 0);
    variable ed    : std_logic;
  begin
    while cyc < n_cyc loop
      wait until rising_edge(clk);
      cyc := cyc + 1;
      if cyc >= 3 then
        for m in 0 to NM - 1 loop
          assert dig(m) = mdg(m)
            report "digest mismatch in mid " & integer'image(m) & " at cycle " & integer'image(cyc)
            severity failure;
        end loop;
        if cyc mod 4 = 0 then
          for i in 0 to NT - 1 loop
            s := (i / NL) mod NSETS;
            if stim(s).dir = '1' then
              eio := ea(i).v8;
              ed  := ea(i).s1;
            else
              eio := far(i, stim(s).b);
              ed  := std_logic(ea(i).s2);
            end if;
            acc_d := fold(acc_d, pack(oa(i)));
            acc_m := fold(acc_m, pack(ea(i)));
            acc_d := fold(acc_d, std_logic_vector'(od(i) & oseen(i) & io_bus(i)));
            acc_m := fold(acc_m, std_logic_vector'(ed & eio & eio));
          end loop;
          assert acc_d = acc_m
            report "output checksum mismatch at cycle " & integer'image(cyc)
            severity failure;
        end if;
      end if;
      -- Digest model uses the values the leaves registered at the previous edge.
      for m in 0 to NM - 1 loop
        if stim(m mod NSETS).run = '1' then
          for l in 0 to NL - 1 loop
            mdg(m) := fold(fold(mdg(m), pack(eb(m * NL + l))), pack(ec(m * NL + l)));
          end loop;
        end if;
      end loop;
      for i in 0 to NT - 1 loop
        s := (i / NL) mod NSETS;
        if cyc <= 1 or stim(s) /= pst(s) then
          w     := mk_w(stim(s).a, stim(s).b, stim(s).sel, stim(s).st, stim(s).mode, i);
          ea(i) := mk_a(w);
          eb(i) := mk_b(w);
          ec(i) := mk_c(w);
        end if;
      end loop;
      pst := stim;
    end loop;
    assert acc_d /= x"00000000" report "checksum never advanced" severity failure;
    report "PASS" severity note;
    done <= true;
    wait;
  end process check;
end architecture sim;
