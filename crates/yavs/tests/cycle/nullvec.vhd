-- Arrays of no elements against the normal mode (`W` replaced by a width; generic `G` is 0): an
-- array of null elements written at an index chosen at run time and read through `resize`, a
-- record with a null field in the two-process style, a null field in `&`, a null variable with an
-- initial value, a null constant, a null slice of a wider input; one indexed by a `natural`, which no
-- operation would check, stays in the event kernel.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_nullvec_u is
  generic (G : natural);
  port (clk : in std_ulogic; x : in std_ulogic_vector(W + 7 downto 0); wp, rp : in natural range 0 to 3; wn : in natural;
        id : out unsigned(7 downto 0); y : out std_ulogic_vector(W - 1 + G downto 0);
        z : out std_ulogic_vector(W + 7 downto 0); cnt : out unsigned(W - 1 downto 0));
end entity;
architecture a of cycle_nullvec_u is
  type ids_t is array (0 to 3) of unsigned(G - 1 downto 0);
  signal ids, ids2 : ids_t := (others => (others => '0'));
  type r_t is record
    cnt : unsigned(W - 1 downto 0);
    tag : std_ulogic_vector(G - 1 downto 0);
  end record;
  signal r, r_next : r_t := ((others => '0'), (others => '0'));
  constant NUL : std_ulogic_vector(G - 1 downto 0) := (others => '0');
begin
  wr : process (clk)
  begin
    if rising_edge(clk) then
      ids(wp) <= unsigned(x(G - 1 downto 0));
      id <= resize(ids(rp), 8) + unsigned(x(7 downto 0));
    end if;
  end process;
  wr2 : process (clk)
  begin
    if rising_edge(clk) then
      ids2(wn) <= unsigned(x(G - 1 downto 0));
    end if;
  end process;
  comb : process (all)
    variable v : r_t;
    variable t : std_ulogic_vector(G - 1 downto 0) := (others => '0');
  begin
    v := r;
    v.cnt := r.cnt + 1;
    t := x(G - 1 downto 0);
    v.tag := t;
    r_next <= v;
  end process;
  reg : process (clk)
  begin
    if rising_edge(clk) then
      r <= r_next;
    end if;
  end process;
  y <= r.tag & std_ulogic_vector(r.cnt);
  z <= x xor (NUL & x);
  cnt <= r.cnt;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_nullvec is
end entity;
architecture t of cycle_nullvec is
  signal clk : std_ulogic := '0';
  signal x, z : std_ulogic_vector(W + 7 downto 0) := (others => '0');
  signal wp, rp : natural range 0 to 3 := 0;
  signal wn : natural := 0;
  signal id : unsigned(7 downto 0);
  signal y : std_ulogic_vector(W - 1 downto 0);
  signal cnt : unsigned(W - 1 downto 0);
begin
  u : entity work.cycle_nullvec_u
    generic map (G => 0)
    port map (clk => clk, x => x, wp => wp, rp => rp, wn => wn, id => id, y => y, z => z, cnt => cnt);
  main : process
    variable s : unsigned(15 downto 0) := x"ACE1";
  begin
    for k in 1 to 20 loop
      s := s(14 downto 0) & (s(15) xor s(13) xor s(12) xor s(10));
      x <= x(W + 6 downto 0) & s(0);
      wp <= k mod 4;
      wn <= (k + 1) mod 4;
      rp <= (k * 3) mod 4;
      wait for 1 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
      report "k=" & integer'image(k) & " id=" & to_hstring(id) & " y=" & to_hstring(y)
        & " z=" & to_hstring(z) & " cnt=" & to_hstring(cnt);
    end loop;
    wait;
  end process;
end architecture;
