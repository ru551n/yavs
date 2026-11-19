-- Expression actuals of ports (LRM 6.5.6.3) and array aggregates without a context (one choice, a
-- range of generics: a subtype's, one against the direction of the index subtype), in the region against the
-- normal mode (`W` replaced by a width): a logic expression, a record aggregate of a field, an
-- integer expression and a vector expression as actuals of `in` ports; elements of constants of
-- two dimensions at indices of unrolled loops.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
package cycle_portact_pkg is
  type s2m_t is record
    ready : std_ulogic;
  end record;
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cycle_portact_pkg.all;
entity cycle_portact_u is
  generic (K : natural);
  port (clk : in std_ulogic; en : in std_ulogic; s2m : in s2m_t; n : in integer range 0 to 15;
        v : in unsigned(W - 1 downto 0); q : out unsigned(W - 1 downto 0) := (others => '0');
        c : out natural);
end entity;
architecture a of cycle_portact_u is
begin
  process (clk)
  begin
    if rising_edge(clk) then
      if en = '1' and s2m.ready = '1' then
        q <= v xor resize(to_unsigned(n, 4), W);
      end if;
      c <= n + 20 * K + to_integer(v(0 downto 0));
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
-- Aggregates whose range is a function of generics: no context gives their bounds.
entity cycle_portact_t is
  generic (N : positive; M : positive);
  port (clk, a, b : in std_ulogic; rdy : in std_ulogic_vector(0 to N - 1);
        seen : out std_ulogic_vector(0 to N - 1); mix : out std_ulogic_vector(M - 1 downto 0));
end entity;
architecture a of cycle_portact_t is
  constant c_n : positive := N;
  subtype bits_t is std_ulogic_vector(0 to c_n - 1);
  signal seen_q : bits_t := (others => '0');
  signal mix_q : std_ulogic_vector(M - 1 downto 0) := (others => '0');
begin
  track : process (clk)
  begin
    if rising_edge(clk) then
      seen_q <= (seen_q or rdy) and not (bits_t'range => a);
      mix_q <= (not (M - 1 downto 1 => a) & b) xor (mix_q(M - 2 downto 0) & mix_q(M - 1));
    end if;
  end process;
  seen <= seen_q;
  mix <= mix_q;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cycle_portact_pkg.all;
entity cycle_portact is
end entity;
architecture t of cycle_portact is
  subtype bits_t is std_ulogic_vector(0 to 5);
  signal clk, a, b : std_ulogic := '0';
  signal rdy : bits_t := (others => '0');
  signal cnt : unsigned(7 downto 0) := (others => '0');
  signal x, y : unsigned(W - 1 downto 0) := (others => '0');
  type qs_t is array (0 to 1) of unsigned(W - 1 downto 0);
  signal q : qs_t;
  type cs_t is array (0 to 1) of natural;
  signal c : cs_t;
  signal seen : bits_t;
  signal mix : std_ulogic_vector(7 downto 0);
  -- Constants of two dimensions, indexed by loop parameters.
  type grid_t is array (0 to 1, 2 downto 0) of natural;
  constant grid : grid_t := ((1, 2, 3), (4, 5, 6));
  type lgrid_t is array (1 to 2, 0 to 3) of std_ulogic;
  constant lgrid : lgrid_t := ("0110", "1001");
  signal hit : std_ulogic_vector(0 to 5) := (others => '0');
  signal lsel : std_ulogic_vector(0 to 7) := (others => '0');
begin
  g : for k in 0 to 1 generate
    u : entity work.cycle_portact_u
      generic map (K => k)
      port map (clk => clk, en => a or b, s2m => (ready => rdy(k)), n => to_integer(cnt) mod 16,
                v => x xor y, q => q(k), c => c(k));
  end generate;
  t : entity work.cycle_portact_t generic map (N => 6, M => 8)
    port map (clk => clk, a => a, b => b, rdy => rdy, seen => seen, mix => mix);
  grids : process (clk)
  begin
    if rising_edge(clk) then
      for r in 0 to 1 loop
        for c in 2 downto 0 loop
          hit(3 * r + c) <= '1' when grid(r, c) = to_integer(cnt) mod 7 else '0';
        end loop;
      end loop;
      for r in 1 to 2 loop
        for c in 0 to 3 loop
          lsel(4 * (r - 1) + c) <= lgrid(r, c) and rdy(c);
        end loop;
      end loop;
    end if;
  end process;
  main : process
    variable r : unsigned(15 downto 0) := x"ACE1";
  begin
    for k in 1 to 30 loop
      r := r(14 downto 0) & (r(15) xor r(13) xor r(12) xor r(10));
      a <= r(0);
      b <= r(3);
      rdy <= std_ulogic_vector(r(5 downto 0));
      cnt <= cnt + 3;
      x <= x(W - 2 downto 0) & r(1) when W > 1 else x xor "1";
      y <= not y when k mod 4 = 0 else y;
      wait for 1 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
      report "k=" & integer'image(k) & " q0=" & to_hstring(q(0)) & " q1=" & to_hstring(q(1))
        & " c=" & integer'image(c(0)) & "," & integer'image(c(1)) & " seen=" & to_string(seen)
        & " mix=" & to_string(mix) & " hit=" & to_string(hit) & " lsel=" & to_string(lsel);
    end loop;
    wait;
  end process;
end architecture;
