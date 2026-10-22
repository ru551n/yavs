-- difftest: top=out_port_collapse std=2008
-- `out` and `buffer` ports that are the only source of their actual (aliases of the actual
-- when ports are collapsed) and the cases that keep their edges: initial values through a
-- chain (the actual starts at the deepest formal's default), parts of a vector, a record
-- through a component, two resolved drivers of the formal, an unresolved formal on a
-- resolved actual, 'driving and 'driving_value, events, 'transaction, 'last_value,
-- 'last_event and 'active read through the formal and the actual, an actual with a second
-- driver, a partially associated formal, an `inout` actual above, a forced actual.
library ieee;
use ieee.std_logic_1164.all;

package out_port_collapse_pkg is
  type rec_t is record
    n : integer;
    b : std_logic_vector(1 downto 0);
  end record;
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.out_port_collapse_pkg.all;

entity out_port_collapse_leaf is
  port (clk : in std_logic;
        q : out std_logic_vector(3 downto 0) := "1010";
        n : out integer := 7;
        r : out rec_t := (5, "01");
        z : out std_logic := 'H';
        u : out std_ulogic := '1';
        b : buffer std_logic_vector(1 downto 0) := "10");
end;
architecture t of out_port_collapse_leaf is
begin
  process (clk)
    variable k : integer := 0;
  begin
    if rising_edge(clk) then
      report "drv=" & to_string(q'driving_value) & " driving=" & boolean'image(q'driving) &
        " n drv=" & integer'image(n'driving_value);
      k := k + 1;
      q <= std_logic_vector(to_unsigned(k, 4));
      n <= k;
      r.n <= k * 10;
      b <= std_logic_vector(unsigned(b) + 1);
      u <= not u;
    end if;
  end process;
  r.b <= b;
  -- Two drivers of the resolved formal.
  z <= 'H' when q(0) = '1' else 'Z';
  z <= 'L' when q(1) = '1' else 'Z';
  process (q, n) begin
    if now > 0 ns then
      report "leaf q=" & to_string(q) & " n=" & integer'image(n) & " q'event=" &
        boolean'image(q'event) & " n'last_value=" & integer'image(n'last_value) & " n'active=" &
        boolean'image(n'active);
    end if;
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;
use work.out_port_collapse_pkg.all;

entity out_port_collapse_mid is
  port (clk : in std_logic;
        q : out std_logic_vector(3 downto 0);
        n : out integer;
        r : out rec_t;
        z : out std_logic;
        u : out std_logic;
        b : buffer std_logic_vector(1 downto 0));
end;
architecture t of out_port_collapse_mid is
  component out_port_collapse_leaf is
    port (clk : in std_logic;
          q : out std_logic_vector(3 downto 0) := "1010";
          n : out integer := 7;
          r : out rec_t := (5, "01");
          z : out std_logic := 'H';
          u : out std_ulogic := '1';
          b : buffer std_logic_vector(1 downto 0) := "10");
  end component;
begin
  l : out_port_collapse_leaf port map (clk, q, n, r, z, u, b);
  process (r, b) begin
    -- 'last_event and 'last_value before the first event differ between the references.
    if now > 0 ns then
      report "mid r.n=" & integer'image(r.n) & " r.b=" & to_string(r.b) & " b=" &
        to_string(b) & " r'last_event=" & time'image(r'last_event);
    end if;
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;

entity out_port_collapse_io is
  port (clk : in std_logic; p : inout std_logic_vector(3 downto 0));
end;
architecture t of out_port_collapse_io is
begin
  -- `p` is fed from above: the leaf's `q` keeps its edge.
  l : entity work.out_port_collapse_leaf port map (clk => clk, q => p);
end;

library ieee;
use ieee.std_logic_1164.all;
use work.out_port_collapse_pkg.all;

entity out_port_collapse is end;
architecture t of out_port_collapse is
  signal clk : std_logic := '0';
  signal q : std_logic_vector(3 downto 0) := "0000";
  signal n : integer := 0;
  signal r : rec_t := (0, "00");
  signal z, u : std_logic := '0';
  signal b : std_logic_vector(1 downto 0);
  signal w : std_logic_vector(7 downto 0) := x"FF";
  signal two : std_logic_vector(3 downto 0);
  signal halves : std_logic_vector(3 downto 0);
  signal lo, hi : std_logic_vector(1 downto 0);
  signal bus_q : std_logic_vector(3 downto 0);
  signal forced : std_logic_vector(3 downto 0);
begin
  m : entity work.out_port_collapse_mid port map (clk, q, n, r, z, u, b);
  -- Part of a vector.
  p : entity work.out_port_collapse_leaf port map (clk => clk, q => w(5 downto 2));
  -- A second driver of the resolved actual.
  s : entity work.out_port_collapse_leaf port map (clk => clk, q => two);
  two <= "ZZZ1";
  -- A partially associated formal.
  h : entity work.out_port_collapse_leaf
    port map (clk => clk, q(1 downto 0) => lo, q(3 downto 2) => hi);
  halves <= hi & lo;
  -- An `inout` actual above.
  i : entity work.out_port_collapse_io port map (clk => clk, p => bus_q);
  -- A forced actual.
  f : entity work.out_port_collapse_leaf port map (clk => clk, q => forced);

  process begin
    report "init q=" & to_string(q) & " n=" & integer'image(n) & " r.n=" &
      integer'image(r.n) & " r.b=" & to_string(r.b) & " z=" & std_logic'image(z) & " u=" &
      std_logic'image(u) & " b=" & to_string(b) & " w=" & to_string(w) & " two=" &
      to_string(two) & " halves=" & to_string(halves) & " bus_q=" & to_string(bus_q) &
      " forced=" & to_string(forced);
    for k in 1 to 4 loop
      wait for 5 ns;
      clk <= '1';
      if k = 2 then
        forced <= force "0110";
      elsif k = 4 then
        forced <= release;
      end if;
      wait for 5 ns;
      clk <= '0';
      wait for 0 ns;
      report "k=" & integer'image(k) & " q=" & to_string(q) & " n=" & integer'image(n) &
        " r.n=" & integer'image(r.n) & " z=" & std_logic'image(z) & " u=" &
        std_logic'image(u) & " b=" & to_string(b) & " w=" & to_string(w) & " two=" &
        to_string(two) & " halves=" & to_string(halves) & " bus_q=" & to_string(bus_q) &
        " forced=" & to_string(forced) & " q'last_event=" & time'image(q'last_event) &
        " n'last_value=" & integer'image(n'last_value);
    end loop;
    assert q = "0100" and n = 4 and r.n = 40 and halves = "0100" severity failure;
    wait;
  end process;

  process (q'transaction) begin
    report "q'transaction q=" & to_string(q) & " active=" & boolean'image(q'active);
  end process;
end;
