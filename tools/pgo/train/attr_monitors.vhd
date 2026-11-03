-- pgo: top=attr_monitors_tb std=2008 stop=8us
-- Testbench-style lanes that lean on implicit signals and timed waits:
-- 'delayed, 'stable, 'quiet, 'transaction, 'last_value, 'last_event, 'event,
-- 'active, 'driving, 'driving_value; setup, hold and pulse-width checkers that
-- stay silent; wait for, wait until ... for, wait on; inertial, transport and
-- reject delays; guarded signals; shared resolved buses; blocks. Random
-- stimulus from math_real and wide vector shifts and rotations.

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use ieee.math_real.all;

package am_pkg is
  type nat_arr is array (natural range <>) of natural;
  type slv8_arr is array (natural range <>) of std_logic_vector(7 downto 0);

  procedure rand_int (variable s1, s2 : inout positive; constant n : in positive;
                      variable r : out natural);
  procedure drive_pulse (signal p : out std_logic; constant w : in time);
  procedure sync_fall (signal c : in std_logic);
  procedure flip (signal t : inout std_logic);
end package am_pkg;

package body am_pkg is
  procedure rand_int (variable s1, s2 : inout positive; constant n : in positive;
                      variable r : out natural) is
    variable x : real;
  begin
    uniform(s1, s2, x);
    r := integer(floor(x * real(n)));
  end procedure rand_int;

  procedure drive_pulse (signal p : out std_logic; constant w : in time) is
  begin
    p <= '1';
    wait for w;
    p <= '0';
  end procedure drive_pulse;

  procedure sync_fall (signal c : in std_logic) is
  begin
    wait until falling_edge(c);
  end procedure sync_fall;

  procedure flip (signal t : inout std_logic) is
  begin
    t <= not t;
  end procedure flip;
end package body am_pkg;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.am_pkg.all;

entity am_lane is
  generic (idx : natural; wid : positive);
  port (
    clk_f   : in    std_logic;
    clk_s   : in    std_logic;
    done    : in    std_logic;
    sc      : in    natural;
    rbus    : inout std_logic_vector(7 downto 0);
    cnt_out : out   natural
  );
end entity am_lane;

architecture rtl of am_lane is
  signal d      : std_logic_vector(7 downto 0) := (others => '0');
  signal d1     : std_logic := '0';
  signal d_tr   : std_logic_vector(7 downto 0) := (others => '0');
  signal d_in   : std_logic_vector(7 downto 0) := (others => '0');
  signal d_rj   : std_logic_vector(7 downto 0) := (others => '0');
  signal pls    : std_logic := '0';
  signal pls_dl : std_logic;
  signal en_g   : std_logic := '0';
  signal gsig   : std_logic bus;
  signal tick   : std_logic := '0';
  signal n_assign : natural := 0;
  signal n_change : natural := 0;
  signal dmatch : boolean;

  disconnect gsig : std_logic after 1 ns;
begin
  -- Delay models of the same data: all must agree with 'delayed.
  d_tr   <= transport d after 3 ns;
  d_in   <= d after 3 ns;
  d_rj   <= reject 1 ns inertial d after 3 ns;
  pls_dl <= pls'delayed(2 ns);

  cmp_blk : block
    generic (tol : natural);
    generic map (tol => 3);
    port (a, b : in std_logic_vector(7 downto 0); ok : out boolean);
    port map (a => d_in, b => d_tr, ok => dmatch);
  begin
    ok <= a = b or tol = 0;
  end block cmp_blk;

  -- Guarded driver: disconnects when the guard is false.
  gblk : block (en_g = '1')
  begin
    gsig <= guarded d1;
  end block gblk;

  -- Data changes only on falling fast-clock edges, after one of several waits.
  stim : process
    type tab_t is array (0 to 63) of natural;
    variable tab : tab_t := (others => 0);
    variable s1  : positive := 11 + idx;
    variable s2  : positive := 977 + 3 * idx;
    variable r   : natural;
    variable i   : natural := 0;
    variable nv  : std_logic_vector(7 downto 0);
  begin
    for j in 0 to 599 loop
      rand_int(s1, s2, 1024, r);
      tab(j mod 64) := (tab(j mod 64) * 31 + r) mod 1024;
    end loop;
    while done = '0' loop
      i := i + 1;
      rand_int(s1, s2, 256, r);
      case tab(i mod 64) mod 4 is
        when 0      => wait for (2 + tab((i + 7) mod 64) mod 9) * 1 ns;
        when 1      => wait until rising_edge(clk_f);
        when 2      => wait until en_g = '1' for 30 ns;
        when others => wait on clk_s for 40 ns;
      end case;
      sync_fall(clk_f);
      nv := std_logic_vector(to_unsigned(r, 8));
      if nv /= d then
        n_change <= n_change + 1;
      end if;
      d        <= nv;
      d1       <= nv(0) xor nv(3);
      n_assign <= n_assign + 1;
    end loop;
    wait;
  end process stim;

  guard_ctl : process
  begin
    sync_fall(clk_f);
    if clk_s = '1' then
      en_g <= '1';
    else
      en_g <= '0';
    end if;
  end process guard_ctl;

  -- Pulses of at least 3 ns with gaps of at least 3 ns.
  gen_pls : process
    variable s1 : positive := 5 + idx;
    variable s2 : positive := 313 + 7 * idx;
    variable r  : natural;
  begin
    wait for (1 + idx mod 5) * 1 ns;
    while done = '0' loop
      rand_int(s1, s2, 7, r);
      wait for (3 + r) * 1 ns;
      rand_int(s1, s2, 6, r);
      drive_pulse(pls, (3 + r) * 1 ns);
    end loop;
    wait;
  end process gen_pls;

  chk_pw : process
    variable t0 : time := 0 ns;
  begin
    wait until pls = '1';
    t0 := now;
    assert pls'last_value = '0' report "pulse rise without low" severity failure;
    wait until pls = '0' for 50 ns;
    assert pls = '0' report "pulse too long" severity failure;
    assert now - t0 >= 3 ns report "pulse too short" severity failure;
    assert pls'last_event = 0 ns and pls'last_value = '1'
      report "pulse end attributes" severity failure;
    assert pls'delayed(1 ns) = '1' report "pulse delayed" severity failure;
  end process chk_pw;

  -- Setup checks on every rising edge.
  chk_su : process (clk_f)
  begin
    if rising_edge(clk_f) and now > 20 ns then
      assert d'stable(1 ns) and d1'stable(1 ns) report "setup violation" severity failure;
      assert (not d'quiet(1 ns)) or d'stable(1 ns) report "quiet implies stable" severity failure;
      assert d'stable(3 ns) = (d'last_event >= 3 ns) report "stable vs last_event" severity failure;
      assert d'quiet(3 ns) = (d'last_active >= 3 ns) report "quiet vs last_active" severity failure;
      assert d_tr = d'delayed(3 ns) report "transport vs delayed" severity failure;
      assert d_in = d_tr and d_rj = d_tr and dmatch report "delay models differ" severity failure;
      assert pls_dl = pls'delayed(2 ns) report "pulse delayed signal" severity failure;
      if en_g = '1' then
        assert gsig = d1 report "guarded value" severity failure;
      else
        assert gsig = 'Z' report "guard release" severity failure;
      end if;
    end if;
  end process chk_su;

  chk_hold : process
  begin
    wait until rising_edge(clk_f);
    wait for 1 ns;
    if now > 20 ns then
      assert d'last_event >= 1 ns and d1'last_event >= 1 ns
        report "hold violation" severity failure;
    end if;
  end process chk_hold;

  -- Shared resolved bus: one lane of the group drives it at a time.
  bus_drv : process (clk_f)
    variable drv  : boolean := false;
    variable last : std_logic_vector(7 downto 0) := (others => 'Z');
  begin
    if rising_edge(clk_f) then
      if now > 20 ns then
        assert not is_x(rbus) report "bus is unknown" severity failure;
        assert rbus'driving and rbus'driving_value = last
          report "bus driving attributes" severity failure;
        if drv then
          assert rbus = last report "bus value" severity failure;
        end if;
      end if;
      drv := (sc mod 4) = (idx mod 4);
      if drv then
        last := d_in;
      else
        last := (others => 'Z');
      end if;
      rbus <= last;
    end if;
  end process bus_drv;

  -- Event, transaction and activity bookkeeping.
  mon_blk : block
    signal n_event : natural := 0;
    signal n_trans : natural := 0;
  begin
    cnt_ev : process (d)
    begin
      if d'event then
        assert d'active and d'last_value /= d report "event attributes" severity failure;
        n_event <= n_event + 1;
      end if;
    end process cnt_ev;

    cnt_tr : process (d'transaction)
    begin
      if now > 0 ns then
        assert d'active report "transaction without activity" severity failure;
        n_trans <= n_trans + 1;
      end if;
    end process cnt_tr;

    tick_p : process (d1)
    begin
      if d1'event then
        assert d1'last_value /= d1 report "d1 last_value" severity failure;
        flip(tick);
      end if;
    end process tick_p;

    fin : process
    begin
      wait until done = '1';
      wait for 200 ns;
      assert n_event = n_change report "event count" severity failure;
      assert n_trans = n_assign report "transaction count" severity failure;
      cnt_out <= n_assign + n_event;
      wait;
    end process fin;
  end block mon_blk;

  -- Wide vector arithmetic on random data, checked by identities.
  wmath : process (clk_s)
    variable s1  : positive := 101 + idx;
    variable s2  : positive := 55 + 5 * idx;
    variable x   : unsigned(wid - 1 downto 0) := (others => '0');
    variable y   : unsigned(wid - 1 downto 0);
    variable sg  : signed(wid - 1 downto 0);
    variable sg2 : signed(wid - 1 downto 0);
    variable sv  : std_logic_vector(wid - 1 downto 0);
    variable t01 : std_logic_vector(wid - 1 downto 0);
    variable r, k, n : natural;
    variable ones : unsigned(wid - 1 downto 0) := (others => '1');
  begin
    if rising_edge(clk_s) then
      for j in 0 to wid / 16 loop
        rand_int(s1, s2, 65536, r);
        x := shift_left(x, 16) or resize(to_unsigned(r, 16), wid);
      end loop;
      rand_int(s1, s2, wid, k);
      rand_int(s1, s2, 40, n);
      y := rotate_right(rotate_left(x, k), k);
      assert y = x report "rotate identity" severity failure;
      y := shift_right(shift_left(x, n), n);
      assert y = (x and shift_right(ones, n)) report "shift mask" severity failure;
      sg  := signed(x);
      sg2 := shift_right(sg, n);
      assert sg2(wid - 1) = sg(wid - 1) report "arithmetic shift sign" severity failure;
      assert shift_left(sg, 0) = sg and rotate_left(sg, wid) = sg
        report "signed identities" severity failure;
      assert to_integer(x(29 downto 0)) = to_integer(to_unsigned(to_integer(x(29 downto 0)), 30))
        report "to_integer roundtrip" severity failure;
      sv := std_logic_vector(x);
      assert not is_x(sv) report "clean vector" severity failure;
      sv(k) := 'X';
      assert is_x(sv) report "x detected" severity failure;
      sv(k) := 'H';
      t01 := to_x01(sv);
      assert t01(k) = '1' and not is_x(t01) report "to_x01 of H" severity failure;
      sv(k) := 'L';
      t01 := to_x01(sv);
      assert t01(k) = '0' report "to_x01 of L" severity failure;
    end if;
  end process wmath;
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use ieee.math_real.all;
use work.am_pkg.all;

entity attr_monitors_tb is
end entity attr_monitors_tb;

architecture sim of attr_monitors_tb is
  constant nlanes : positive := 36;
  constant ngroup : positive := nlanes / 4;
  constant t_end  : time := 7 us;

  signal clk_f : std_logic := '0';
  signal clk_s : std_logic := '0';
  signal done  : std_logic := '0';
  signal sc    : natural := 0;
  signal rbus  : slv8_arr(0 to ngroup - 1);
  signal cnts  : nat_arr(0 to nlanes - 1);
begin
  fast_clk : process
  begin
    while done = '0' loop
      wait for 2 ns;
      clk_f <= not clk_f;
    end loop;
    wait;
  end process fast_clk;

  slow_clk : process
  begin
    while done = '0' loop
      wait for 32 ns;
      clk_s <= not clk_s;
    end loop;
    wait;
  end process slow_clk;

  sel_cnt : process (clk_s)
  begin
    if rising_edge(clk_s) then
      sc <= sc + 1;
    end if;
  end process sel_cnt;

  lanes : for i in 0 to nlanes - 1 generate
    u : entity work.am_lane
      generic map (idx => i, wid => 70 + (i * 13) mod 131)
      port map (clk_f => clk_f, clk_s => clk_s, done => done, sc => sc,
                rbus => rbus(i / 4), cnt_out => cnts(i));
  end generate lanes;

  -- Statistics of the random generator over a long run of draws.
  rng_stats : process
    type bins_t is array (0 to 15) of natural;
    variable bins : bins_t := (others => 0);
    variable s1   : positive := 1234;
    variable s2   : positive := 4321;
    variable x    : real;
    variable sum  : real := 0.0;
    variable b    : natural;
  begin
    for j in 1 to 30000 loop
      uniform(s1, s2, x);
      b := integer(floor(x * 16.0));
      bins(b) := bins(b) + 1;
      sum := sum + x;
    end loop;
    for j in bins'range loop
      assert bins(j) > 1600 and bins(j) < 2150
        report "uniform bin " & integer'image(j) & " count " & integer'image(bins(j))
        severity failure;
    end loop;
    assert sum / 30000.0 > 0.49 and sum / 30000.0 < 0.51 report "uniform mean" severity failure;
    wait;
  end process rng_stats;

  control : process
    variable total : natural := 0;
  begin
    wait for t_end;
    done <= '1';
    wait for 400 ns;
    for i in cnts'range loop
      assert cnts(i) > 20 report "lane " & integer'image(i) & " did not run" severity failure;
      total := total + cnts(i);
    end loop;
    assert total > 1000 report "too little activity" severity failure;
    report "PASS" severity note;
    wait;
  end process control;
end architecture sim;
