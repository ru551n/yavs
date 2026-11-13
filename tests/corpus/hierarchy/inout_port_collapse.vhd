-- difftest: top=inout_port_collapse std=2008 ignore=order ref=hand
-- `inout` ports through three levels of instances (aliases of their actuals when ports are
-- collapsed) and the cases that keep their edges: a leaf driven on both sides, two drivers
-- inside plus one outside (nested resolution), parts of a vector driven at two levels, an
-- undriven inout leaf contributing its default ('U', and 'Z' with a '-' outside), a formal
-- whose default differs from its actual's initial value, unresolved integer, std_ulogic and
-- boolean leaves, an undriven unresolved leaf, an open inout port; values, 'event, 'active,
-- 'last_value, 'transaction, 'driving and 'driving_value through the formals and the actual,
-- and the delta at which each level sees a change.
-- ignore=order: the readers of the three levels report in the same delta.
-- ref=hand: the output of the second reference simulator with the deltas of the first, which
-- differs in three places: `'driving_value` of the port is its effective value there (LRM
-- 16.2.4: the value of the process's driver, `1` and `-`), `p'transaction` of the formal does
-- not toggle when only the actual is active (LRM 14.7.3.1: a formal is active when its actual
-- is, at 2, 4 and 7 ns), and `q'last_value` per element (catalogue `last-value-composite`).
library ieee;
use ieee.std_logic_1164.all;

entity iopc_l3 is
  port (p   : inout std_logic;
        two : inout std_logic;
        q   : inout std_logic_vector(3 downto 0);
        w   : inout std_logic;
        wz  : inout std_logic := 'Z';
        i1  : inout std_logic := '1';
        n   : inout integer := 0;
        m   : inout integer := 5;
        u   : inout std_ulogic;
        bo  : inout boolean;
        o   : inout std_logic_vector(1 downto 0) := "HL");
end entity;

architecture a of iopc_l3 is
begin
  drv_p : process
  begin
    p <= 'Z';
    wait for 1 ns;
    p <= '1';
    wait for 2 ns;
    report "l3 drv p'driving_value=" & to_string(p'driving_value) & " p'driving="
      & to_string(p'driving) & " p=" & to_string(p);
    p <= 'Z';
    wait for 2 ns;
    p <= '-';
    wait for 1 ns;
    report "l3 drv p'driving_value=" & to_string(p'driving_value) & " p=" & to_string(p);
    p <= '0';
    wait;
  end process;

  d1 : process
  begin
    two <= '1';
    wait for 2 ns;
    two <= 'Z';
    wait;
  end process;
  d2 : process
  begin
    two <= 'Z';
    wait for 1 ns;
    two <= '0';
    wait for 2 ns;
    two <= 'H';
    wait;
  end process;

  q(3 downto 2) <= "ZZ";
  qlo : process
  begin
    q(1 downto 0) <= "10";
    wait for 2 ns;
    q(1 downto 0) <= "01";
    wait;
  end process;

  i1d : process
  begin
    wait for 6 ns;
    i1 <= '0';
    wait;
  end process;

  nd : process
  begin
    n <= 1;
    wait for 2 ns;
    n <= 2;
    wait;
  end process;

  u <= '1' after 1 ns;
  bo <= true after 3 ns;
  o(0) <= '1';

  rd : process
  begin
    wait on p, two, q, w, wz, i1, n, m, u, bo;
    report "l3 p=" & to_string(p) & " p'event=" & to_string(p'event) & " p'last_value="
      & to_string(p'last_value) & " two=" & to_string(two) & " q=" & to_string(q)
      & " w=" & to_string(w) & " wz=" & to_string(wz) & " i1=" & to_string(i1)
      & " n=" & integer'image(n) & " m=" & integer'image(m) & " u=" & to_string(u)
      & " bo=" & boolean'image(bo);
  end process;

  tr : process
  begin
    wait on p'transaction;
    report "l3 p'transaction p=" & to_string(p) & " p'active=" & to_string(p'active)
      & " p'event=" & to_string(p'event);
  end process;

  init : process
  begin
    report "l3 init p=" & to_string(p) & " i1=" & to_string(i1) & " n=" & integer'image(n)
      & " m=" & integer'image(m) & " w=" & to_string(w) & " o=" & to_string(o);
    wait for 9 ns;
    assert o = "H1" report "o=" & to_string(o) severity failure;
    assert m = 5 report "m=" & integer'image(m) severity failure;
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity iopc_l2 is
  port (p   : inout std_logic;
        two : inout std_logic;
        q   : inout std_logic_vector(3 downto 0);
        w   : inout std_logic;
        wz  : inout std_logic := 'Z';
        i1  : inout std_logic;
        n   : inout integer := 0;
        m   : inout integer := 0;
        u   : inout std_ulogic;
        bo  : inout boolean);
end entity;

architecture a of iopc_l2 is
begin
  q(3 downto 2) <= "1Z";
  q(1 downto 0) <= "ZZ";

  l3 : entity work.iopc_l3
    port map (p => p, two => two, q => q, w => w, wz => wz, i1 => i1, n => n, m => m,
              u => u, bo => bo, o => open);

  rd : process
  begin
    wait on p, two, q;
    report "l2 p=" & to_string(p) & " p'event=" & to_string(p'event) & " two="
      & to_string(two) & " q=" & to_string(q) & " q'last_value=" & to_string(q'last_value);
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity iopc_l1 is
  port (p   : inout std_logic;
        two : inout std_logic;
        q   : inout std_logic_vector(3 downto 0);
        w   : inout std_logic;
        wz  : inout std_logic;
        i1  : inout std_logic;
        n   : inout integer := 0;
        m   : inout integer := 0;
        u   : inout std_ulogic;
        bo  : inout boolean);
end entity;

architecture a of iopc_l1 is
begin
  l2 : entity work.iopc_l2
    port map (p => p, two => two, q => q, w => w, wz => wz, i1 => i1, n => n, m => m,
              u => u, bo => bo);
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity inout_port_collapse is
end entity;

architecture a of inout_port_collapse is
  signal p, two, w, wz, i1 : std_logic;
  signal q : std_logic_vector(3 downto 0);
  signal n, m : integer := 0;
  signal u : std_ulogic;
  signal bo : boolean;
begin
  l1 : entity work.iopc_l1
    port map (p => p, two => two, q => q, w => w, wz => wz, i1 => i1, n => n, m => m,
              u => u, bo => bo);

  drv_p : process
  begin
    p <= 'Z';
    wait for 2 ns;
    p <= '0';
    wait for 2 ns;
    p <= 'Z';
    wait for 3 ns;
    p <= 'L';
    wait;
  end process;

  two <= 'Z', 'L' after 4 ns;
  w <= '1';
  wz <= '-', '1' after 2 ns;
  i1 <= 'Z';

  rd : process
  begin
    wait on p, two, q, w, wz, i1, n, m, u, bo;
    report "top p=" & to_string(p) & " p'event=" & to_string(p'event) & " p'last_value="
      & to_string(p'last_value) & " p'active=" & to_string(p'active) & " two="
      & to_string(two) & " q=" & to_string(q) & " w=" & to_string(w) & " wz="
      & to_string(wz) & " i1=" & to_string(i1) & " n=" & integer'image(n) & " m="
      & integer'image(m) & " u=" & to_string(u) & " bo=" & boolean'image(bo);
  end process;

  tr : process
  begin
    wait on p'transaction;
    report "top p'transaction p=" & to_string(p) & " p'active=" & to_string(p'active);
  end process;

  check : process
  begin
    report "top init p=" & to_string(p) & " two=" & to_string(two) & " i1=" & to_string(i1)
      & " n=" & integer'image(n) & " m=" & integer'image(m) & " w=" & to_string(w)
      & " wz=" & to_string(wz);
    wait for 9 ns;
    assert p = '0' report "p=" & to_string(p) severity failure;
    assert two = 'W' report "two=" & to_string(two) severity failure;
    assert q = "1Z01" report "q=" & to_string(q) severity failure;
    assert w = 'U' report "w=" & to_string(w) severity failure;
    assert wz = '1' report "wz=" & to_string(wz) severity failure;
    assert i1 = '0' report "i1=" & to_string(i1) severity failure;
    assert n = 2 and m = 5 report "n m" severity failure;
    assert u = '1' and bo report "u bo" severity failure;
    report "done";
    wait;
  end process;
end architecture;
