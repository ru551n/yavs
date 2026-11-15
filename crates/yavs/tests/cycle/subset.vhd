-- The subset checker of cycle mode: one process per reason a process stays outside the subset
-- (each line named by `subset_reasons_are_reported_with_their_code_and_position`), behavioural
-- processes, and processes of every shape that fits. Only the verdicts are tested
-- (`YAVS_CYCLE_VERDICTS=1`): the design is not run.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity sub is
  generic (WIDE : boolean := false);
  port (clk, rst, a, b, en : in std_ulogic; v8 : in std_ulogic_vector(7 downto 0));
end entity;

architecture rtl of sub is
  type counter is protected
    procedure inc;
  end protected;
  type counter is protected body
    variable n : natural := 0;
    procedure inc is
    begin
      n := n + 1;
    end procedure;
  end protected body;
  signal q1, q2, q3, q4, q5, q6, q7, q8, q9, q10, q11, q12, q13, q14 : std_ulogic;
  signal x, y, gclk, rr : std_ulogic;
  signal s : std_logic;
  signal rs : real := 0.0;
  signal ri, ri2 : integer;
  -- Shapes that fit.
  signal f1, f2, f3, f4, f5, f6, f7, f8, f9, f10, f11, f12, f13, f14 : std_ulogic;
  signal c8 : std_ulogic_vector(7 downto 0);
  signal pc2 : std_ulogic_vector(7 downto 0);
  signal pc4 : std_ulogic_vector(3 downto 0);
  type pair is record
    lo, hi : std_ulogic_vector(3 downto 0);
  end record;
  signal r, rin : pair;
  signal g1, g2, g3, g4, g5, g6, g7, w, z, e1, e2, e3 : std_ulogic;
  type st_t is (IDLE, RUN, DONE);
  function prev(signal x : std_ulogic) return std_ulogic is
  begin
    return x'last_value;
  end function;
  signal e4, dclk, f15 : std_ulogic := '0';
  signal f16 : std_ulogic_vector(3 downto 0);
  signal h1, h2, h3, h4, h5, h6, h7, h8, h9, h10 : std_ulogic;
  signal gs : std_logic bus;
  function pick_one(v : std_ulogic_vector) return std_ulogic is
  begin
    return v(v'left);
  end function;
  subtype own_logic is pick_one std_ulogic;
  signal own : own_logic;
  function half_known(n : natural) return boolean is
    variable v : std_ulogic_vector(3 downto 0);
  begin
    if n > 1 then
      return v(n) = '1';
    end if;
    return false;
  end function;
  function outside(x : std_ulogic) return std_ulogic;
  attribute foreign of outside : function is "VHPIDIRECT outside";
  shared variable sh : counter;
  signal g9 : std_ulogic_vector(2 downto 0);
  function wide_mode(x : natural) return boolean is
  begin
    return x > 4;
  end function;
  function count_to(n : natural) return natural is
    variable c : natural := 0;
  begin
    for i in 1 to n loop
      c := c + 1;
    end loop;
    return c;
  end function;
begin
  -- Outside the subset.
  wait_on : process
  begin
    wait on a;
    q1 <= a;
  end process;

  delayed : process (a)
  begin
    q2 <= a after 1 ns;
  end process;

  history : process (a)
  begin
    q3 <= a'last_value;
  end process;

  real_var : process (clk)
    variable acc : real := 0.0;
  begin
    if rising_edge(clk) then
      acc := acc + 1.0;
    end if;
  end process;

  protected_var : process (clk)
    variable cnt : counter;
  begin
    if rising_edge(clk) then
      cnt.inc;
    end if;
  end process;

  late : postponed process (a)
  begin
    q4 <= a;
  end process;

  changes_state : process (a)
    variable n : natural := 0;
    impure function bump return natural is
    begin
      n := n + 1;
      return n;
    end function;
  begin
    ri <= bump;
  end process;

  high_z : process (a)
  begin
    q5 <= 'Z';
  end process;

  latch : process (a, en)
  begin
    if en = '1' then
      q6 <= a;
    end if;
  end process;

  state : process (a)
    variable v : std_ulogic;
  begin
    q7 <= v;
    v := a;
  end process;

  incomplete : process (a)
  begin
    q8 <= a and b;
  end process;

  loop_x : process (y)
  begin
    x <= not y;
  end process;

  loop_y : process (x, en)
  begin
    y <= x and en;
  end process;

  outside_edge : process (clk)
  begin
    q9 <= a;
    if rising_edge(clk) then
      q10 <= a;
    end if;
  end process;

  drive_1 : process (a)
  begin
    s <= a;
  end process;

  drive_2 : process (b)
  begin
    s <= b;
  end process;

  gate : process (clk, en)
  begin
    gclk <= clk and en;
  end process;

  on_gated : process (gclk)
  begin
    if rising_edge(gclk) then
      q11 <= a;
    end if;
  end process;

  reset_maker : process (clk)
  begin
    if rising_edge(clk) then
      rr <= rst;
    end if;
  end process;

  made_reset : process (clk, rr)
  begin
    if rr = '1' then
      q12 <= '0';
    elsif rising_edge(clk) then
      q12 <= a;
    end if;
  end process;

  external : process (clk)
  begin
    if rising_edge(clk) then
      q13 <= <<signal .sub_tb.marker : std_ulogic>>;
    end if;
  end process;

  real_read : process (rs)
  begin
    ri2 <= integer(rs * 2.0);
  end process;

  -- Shapes that fit.
  clocked : process (clk)
  begin
    if rising_edge(clk) then
      f1 <= a;
    end if;
  end process;

  falling : process (clk)
  begin
    if falling_edge(clk) then
      f2 <= a;
    end if;
  end process;

  legacy : process (clk)
  begin
    if clk'event and clk = '1' then
      f3 <= a;
    end if;
  end process;

  enabled : process (clk)
  begin
    if rising_edge(clk) and en = '1' then
      f4 <= a;
    end if;
  end process;

  wait_edge : process
  begin
    wait until rising_edge(clk);
    f5 <= a;
  end process;

  wait_enabled : process
  begin
    wait until rising_edge(clk) and en = '1';
    f6 <= a;
  end process;

  async : process (clk, rst)
  begin
    if rst = '1' then
      f7 <= '0';
    elsif rising_edge(clk) then
      f7 <= a;
    end if;
  end process;

  async_two : process (clk, rst, en)
  begin
    if rst = '1' or en = '0' then
      f8 <= '0';
    elsif rising_edge(clk) then
      f8 <= a;
    end if;
  end process;

  comb_all : process (all)
    variable t : std_ulogic;
  begin
    t := a xor b;
    f9 <= t;
  end process;

  comb_if : process (a, b, en)
  begin
    if en = '1' then
      f10 <= a;
    else
      f10 <= b;
    end if;
    -- Not evaluated in cycle mode, and no reason to stay out.
    assert a = a report "never" severity failure;
  end process;

  f11 <= a and b;
  f12 <= a when en = '1' else b;
  with en select f13 <= a when '1', b when others;
  f14 <= a when rising_edge(clk);
  assert en /= 'X' report "concurrent, not evaluated";

  -- Static loops: every element assigned; levels feeding the next in one process.
  levels : process (all)
  begin
    for i in 0 to 7 loop
      pc2(i) <= v8(i) xor v8(7 - i);
    end loop;
    for i in 0 to 3 loop
      pc4(i) <= pc2(2 * i) and pc2(2 * i + 1);
    end loop;
    c8 <= (others => '0');
    if WIDE then
      c8 <= v8;
    else
      for i in pc4'range loop
        c8(i) <= pc4(i);
      end loop;
    end if;
  end process;

  -- The two-process record style.
  comb_rec : process (r, v8)
    variable w : pair;
  begin
    w := r;
    w.lo := v8(3 downto 0);
    rin <= w;
  end process;

  reg_rec : process (clk)
  begin
    if rising_edge(clk) then
      r <= rin;
    end if;
  end process;

  -- More clocked shapes outside the subset.
  before_wait : process
  begin
    g1 <= a;
    wait until rising_edge(clk);
    g2 <= a;
  end process;

  clock_unlisted : process (a)
  begin
    if rising_edge(clk) then
      g3 <= a;
    end if;
  end process;

  reset_unlisted : process (clk)
  begin
    if rst = '1' then
      g4 <= '0';
    elsif rising_edge(clk) then
      g4 <= a;
    end if;
  end process;

  reset_reads : process (clk, rst)
  begin
    if rst = '1' then
      g5 <= b;
    elsif rising_edge(clk) then
      g5 <= a;
    end if;
  end process;

  on_without_clock : process
  begin
    wait on a until rising_edge(clk);
    g6 <= a;
  end process;

  -- `w` is assigned only when the loop was not left: it depends on `z`, a loop.
  early_exit : process (all)
  begin
    w <= '0';
    for i in 0 to 3 loop
      if a = '1' then
        if z = '1' then
          exit;
        end if;
      end if;
      w <= a;
    end loop;
  end process;

  from_w : process (w)
  begin
    z <= w;
  end process;

  -- History read inside a function.
  called_history : process (a)
  begin
    g7 <= prev(a);
  end process;

  -- Loops over an enumeration, with a static compare of the parameter.
  enum_loop : process (all)
  begin
    e1 <= '0';
    for s in st_t loop
      if s = RUN then
        e1 <= e2;
      end if;
    end loop;
  end process;

  from_e1 : process (e1)
  begin
    e2 <= e1;
  end process;

  enum_only : process (all)
  begin
    for s in st_t loop
      if s = DONE then
        e3 <= a;
      end if;
    end loop;
  end process;

  -- A clock made by a register: the process using it stays out.
  divider : process (clk)
  begin
    if rising_edge(clk) then
      dclk <= not dclk;
    end if;
  end process;

  on_divided : process (dclk)
  begin
    if rising_edge(dclk) then
      q14 <= a;
    end if;
  end process;

  shared_call : process (clk)
  begin
    if rising_edge(clk) then
      sh.inc;
    end if;
  end process;

  -- Pure functions of constants decide a branch and a loop bound.
  static_call : process (all)
  begin
    if wide_mode(3) then
      e4 <= a;
    end if;
    for i in 0 to count_to(3) - 1 loop
      g9(i) <= a;
    end loop;
  end process;

  -- A procedure of a clocked process that changes the process's own state.
  clocked_proc : process (clk)
    variable cnt : natural := 0;
    procedure kick is
    begin
      cnt := cnt + 1;
      f15 <= a;
    end procedure;
  begin
    if rising_edge(clk) then
      kick;
    end if;
  end process;

  -- A function the evaluation gives up on decides nothing: the branch stays live.
  give_up : process (a)
  begin
    if half_known(3) then
      h1 <= a;
    end if;
  end process;

  -- Past the statement budget the loop parameter is unknown, not its last value.
  long_loop : process (a)
  begin
    for i in 0 to 99999 loop
      if i = 99999 then
        h2 <= a;
      end if;
    end loop;
  end process;

  transport_delay : process (a)
  begin
    h3 <= transport a;
  end process;

  reject_limit : process (a)
  begin
    h4 <= reject 1 ns inertial a;
  end process;

  waveform : process (a)
  begin
    h5 <= a, not a after 2 ns;
  end process;

  guarding : block (en = '1')
  begin
    h6 <= guarded a;
  end block;

  calls_foreign : process (a)
  begin
    h7 <= outside(a);
  end process;

  with_else : process (clk)
  begin
    if rising_edge(clk) then
      h8 <= a;
    else
      h8 <= b;
    end if;
  end process;

  nested_wait : process
  begin
    if en = '1' then
      wait until rising_edge(clk);
    end if;
    h9 <= a;
  end process;

  -- Each round moves a dependence one variable further: more rounds than the analysis runs.
  slow_loop : process (all)
    variable c : std_ulogic_vector(29 downto 0);
  begin
    c := (others => '0');
    while en = '1' loop
      c(29) := c(28); c(28) := c(27); c(27) := c(26); c(26) := c(25);
      c(25) := c(24); c(24) := c(23); c(23) := c(22); c(22) := c(21);
      c(21) := c(20); c(20) := c(19); c(19) := c(18); c(18) := c(17);
      c(17) := c(16); c(16) := c(15); c(15) := c(14); c(14) := c(13);
      c(13) := c(12); c(12) := c(11); c(11) := c(10); c(10) := c(9);
      c(9) := c(8); c(8) := c(7); c(7) := c(6); c(6) := c(5);
      c(5) := c(4); c(4) := c(3); c(3) := c(2); c(2) := c(1);
      c(1) := c(0); c(0) := a;
    end loop;
    h10 <= c(29);
  end process;

  drives_bus : process (a)
  begin
    gs <= a;
  end process;

  drives_resolved : process (a)
  begin
    own <= a;
  end process;

  -- Elements of a variable no path assigns keep their initial value: no state.
  partial_var : process (all)
    variable t : std_ulogic_vector(3 downto 0) := "0000";
  begin
    t(0) := a;
    t(1) := b;
    f16 <= t;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

package subset_pkg is
  type duo is record
    a, b : std_ulogic;
  end record;
  type uno is record
    a : std_ulogic;
  end record;
end package;

library ieee;
use ieee.std_logic_1164.all;
use work.subset_pkg.all;

-- A record port associated field by field: its elements are other objects' leaves.
entity by_field is
  port (i : in duo; o : out std_ulogic; j : in uno; p : out std_ulogic);
end entity;

architecture rtl of by_field is
begin
  pass : process (i)
  begin
    o <= i.a;
  end process;

  -- `j` becomes an alias of its actual's leaf, with the actual's record path (none).
  pass_one : process (j)
  begin
    p <= j.a;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

-- A port on a slice of its actual: its elements are the actual's, from an offset.
entity half is
  generic (BAD : boolean);
  port (x : in std_ulogic_vector(7 downto 0); d : out std_ulogic_vector(7 downto 0));
end entity;

architecture rtl of half is
begin
  good_bits : if not BAD generate
    bits : process (all)
    begin
      for i in 0 to 7 loop
        d(i) <= x(i);
      end loop;
    end process;
  end generate;
  bad_bits : if BAD generate
    bits : process (all)
    begin
      for i in 0 to 6 loop
        d(i) <= x(i);
      end loop;
      if x(0) = '1' then
        d(7) <= '1';
      end if;
    end process;
  end generate;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use std.textio.all;

entity sub_tb is
end entity;

architecture tb of sub_tb is
  signal clk, rst, a, b, en, marker : std_ulogic := '0';
  signal v8 : std_ulogic_vector(7 downto 0) := x"00";
  signal bus16 : std_ulogic_vector(15 downto 0);
  signal fx, fy, fo, gx, go : std_ulogic;
  procedure tick(signal c : out std_ulogic) is
  begin
    c <= '1';
    wait for 1 ns;
    c <= '0';
  end procedure;
begin
  dut : entity work.sub port map (clk, rst, a, b, en, v8);
  upper : entity work.half generic map (false) port map (v8, bus16(15 downto 8));
  lower : entity work.half generic map (true) port map (v8, bus16(7 downto 0));
  -- A loop through the field of the record port: `pass` reads `fx` through `i.a`.
  fields : entity work.by_field port map (i.a => fx, i.b => fy, o => fo, j.a => gx, p => go);
  back : process (fo)
  begin
    fx <= fo;
  end process;
  back_one : process (go)
  begin
    gx <= go;
  end process;

  timed : process
  begin
    wait for 1 ns;
  end process;

  forever : process
  begin
    a <= '1';
    wait;
  end process;

  two_waits : process
  begin
    wait on b;
    wait on en;
  end process;

  logger : process (clk)
    variable l : line;
  begin
    if rising_edge(clk) then
      write(l, string'("tick"));
    end if;
  end process;

  waits_inside : process
  begin
    tick(clk);
  end process;
end architecture;
