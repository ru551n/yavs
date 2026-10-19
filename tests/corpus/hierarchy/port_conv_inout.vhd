-- difftest: top=port_conv_inout std=2008 ref=hand
-- Conversions on both sides of an `inout` association (#238, LRM 6.5.7.1): the actual
-- part converts the actual to the formal's type on the way in, the formal part converts
-- the formal to the actual's type on the way out. Scalar and record ports; calls
-- are in `process/call_conversions`.
-- ref=hand: one reference simulator crashes on the scalar port and starts the record
-- port from its default instead of the converted actual; the other gives these results
-- except that `b` does not follow the actual when only the second driver changes it
-- (1.5 ns and 2.5 ns), which 14.7.7.3 requires: the effective value of an inout formal
-- is the actual part's conversion of the actual's effective value.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
package port_conv_inout_pkg is
  type pair is record
    hi : std_logic_vector(3 downto 0);
    lo : integer range 0 to 15;
  end record;
  type wide is record
    n : natural;
    ok : boolean;
  end record;
  function to_slv(i : integer) return std_logic_vector;
  function to_int(v : std_logic_vector) return integer;
  function to_wide(p : pair) return wide;
  function to_pair(w : wide) return pair;
end package;
package body port_conv_inout_pkg is
  function to_slv(i : integer) return std_logic_vector is
  begin
    return std_logic_vector(to_unsigned(i mod 16, 4));
  end function;
  function to_int(v : std_logic_vector) return integer is
  begin
    return to_integer(unsigned(to_01(v)));
  end function;
  function to_wide(p : pair) return wide is
  begin
    return (n => to_integer(unsigned(to_01(p.hi))) * 16 + p.lo, ok => p.hi /= "0000");
  end function;
  function to_pair(w : wide) return pair is
  begin
    return (hi => std_logic_vector(to_unsigned(w.n / 16 mod 16, 4)), lo => w.n mod 16);
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.port_conv_inout_pkg.all;
entity port_conv_inout_sub is
  port (
    b : inout integer;
    w : inout wide
  );
end entity;
architecture a of port_conv_inout_sub is
begin
  process
  begin
    -- Initialisation (14.7.5.2): the actuals' driving values are the formal parts'
    -- conversions of the formals' defaults, the formals' effective values the actual
    -- parts' conversions of those.
    report "sub init b " & integer'image(b) & " w " & integer'image(w.n) & " "
      & boolean'image(w.ok);
    assert b = 0 and w.n = 0 and not w.ok;
    b <= 5;
    w <= (n => 39, ok => false);
    wait for 1 ns;
    report "sub b " & integer'image(b) & " w " & integer'image(w.n) & " "
      & boolean'image(w.ok);
    assert b = 5 and w.n = 39 and w.ok;
    b <= b + 4;
    w <= (n => w.n + 17, ok => false);
    wait for 2 ns;
    report "sub b " & integer'image(b) & " w " & integer'image(w.n) & " "
      & boolean'image(w.ok);
    assert b = 9 and w.n = 56 and w.ok;
    wait;
  end process;

  process
  begin
    wait on b;
    report "sub b event " & integer'image(b);
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.port_conv_inout_pkg.all;
entity port_conv_inout is end;
architecture t of port_conv_inout is
  signal t : std_logic_vector(3 downto 0) := "ZZZZ";
  signal p : pair := ("0001", 7);
begin
  u : entity work.port_conv_inout_sub port map (
    to_slv(b) => to_int(t),
    to_pair(w) => to_wide(p));

  -- A second driver of the resolved actual: `Z` lets the port's value through, a `1`
  -- against the port's `0` makes the actual `X`, which `to_int` reads as 0.
  t <= "ZZZZ", "ZZ1Z" after 1500 ps, "ZZZZ" after 2500 ps;

  process
  begin
    wait for 0 ns;
    wait for 0 ns;
    report "t " & to_string(t) & " p " & to_string(p.hi) & " " & integer'image(p.lo);
    assert t = "0101" and p.hi = "0010" and p.lo = 7;
    wait for 1 ns;
    wait for 0 ns;
    wait for 0 ns;
    report "t " & to_string(t) & " p " & to_string(p.hi) & " " & integer'image(p.lo);
    assert t = "1001" and p.hi = "0011" and p.lo = 8;
    wait for 1 ns;
    report "t " & to_string(t);
    assert t = "10X1";
    wait for 1500 ps;
    report "t " & to_string(t);
    assert t = "1001";
    wait;
  end process;
end architecture;
