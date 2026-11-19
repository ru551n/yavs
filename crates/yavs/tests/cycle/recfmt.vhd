-- Fixed-point formats as records of scalars (`W` replaced by a width): a function given formats
-- sizes its locals by functions of them, a local format made by an aggregate and by a function
-- returning a record and chosen by comparing records, as the Open Logic fixed-point package
-- does; against the normal mode.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
package cycle_recfmt_pkg is
  type fmt_t is record
    s : natural range 0 to 1;
    i : integer;
    f : integer;
  end record;
  constant null_c : fmt_t := (0, 0, -1);
  function width(f : fmt_t) return natural;
  function choose(c : boolean; a, b : fmt_t) return fmt_t;
  function wider(a : fmt_t; b : fmt_t) return fmt_t;
  function grow(a : std_logic_vector; a_fmt : fmt_t; r_fmt : fmt_t) return std_logic_vector;
end package;
package body cycle_recfmt_pkg is
  function width(f : fmt_t) return natural is
  begin
    return f.s + f.i + f.f;
  end function;
  function choose(c : boolean; a, b : fmt_t) return fmt_t is
  begin
    if c then
      return a;
    end if;
    return b;
  end function;
  function wider(a : fmt_t; b : fmt_t) return fmt_t is
  begin
    return (s => 1, f => a.f, i => maximum(a.i, b.i) + 1);
  end function;
  function grow(a : std_logic_vector; a_fmt : fmt_t; r_fmt : fmt_t) return std_logic_vector is
    constant a_c : std_logic_vector(width(a_fmt) - 1 downto 0) := a;
    constant mid_c : fmt_t := (r_fmt.s, r_fmt.i, a_fmt.f);
    constant big_c : fmt_t := wider(choose(r_fmt = null_c, a_fmt, mid_c), a_fmt);
    constant k_c : fmt_t := choose(mid_c = (r_fmt.s, r_fmt.i, a_fmt.f) and a_fmt /= null_c, a_fmt, r_fmt);
    variable v : unsigned(width(big_c) - 1 downto 0);
  begin
    assert width(big_c) > width(r_fmt) report "too narrow" severity failure;
    v := resize(unsigned(a_c), width(big_c)) + (mid_c.i * 3 + mid_c.f) + big_c.i * 5 + k_c.i * 7;
    return std_logic_vector(v(width(r_fmt) - 1 downto 0));
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.cycle_recfmt_pkg.all;
entity cycle_recfmt_u is
  generic (a_fmt_g, r_fmt_g : fmt_t);
  port (a : in std_logic_vector; r : out std_logic_vector);
end entity;
architecture a of cycle_recfmt_u is
  constant a_fmt_c : fmt_t := (a_fmt_g.s, a_fmt_g.i, a_fmt_g.f);
begin
  r <= grow(a, a_fmt_c, r_fmt_g);
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cycle_recfmt_pkg.all;
entity cycle_recfmt is
end entity;
architecture t of cycle_recfmt is
  constant a_fmt_c : fmt_t := (1, W, 3);
  constant r_fmt_c : fmt_t := (s => 1, i => W + 2, f => 3);
  signal a : std_logic_vector(width(a_fmt_c) - 1 downto 0) := (others => '0');
  signal r : std_logic_vector(width(r_fmt_c) - 1 downto 0);
begin
  u : entity work.cycle_recfmt_u generic map (a_fmt_c, r_fmt_c) port map (a, r);
  main : process
  begin
    for k in 1 to 8 loop
      a <= std_logic_vector(unsigned(a) + 37 * k);
      wait for 5 ns;
      report "k=" & integer'image(k) & " r=" & to_hstring(r);
    end loop;
    wait;
  end process;
end architecture;
