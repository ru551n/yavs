-- yavs-test: std=2008
-- expect: E1100 24:31 hx is not declared
-- expect: E1100 25:14 hz is not declared
-- expect: E1100 26:9 hq is not declared
-- expect: E1100 27:12 hw is not declared
-- expect: E1100 28:12 hv is not declared
-- help: similar names exist
-- LRM 12.3, 12.5 (an actual with no visible declaration is the error, not the overload set of the call)
library ieee;
use ieee.std_logic_1164.all;
use std.textio.all;
entity e is end entity;
architecture a of e is
  procedure p (x : integer; y : integer) is begin null; end procedure;
  procedure p (x : bit; y : integer) is begin null; end procedure;
  procedure one (x : integer) is begin null; end procedure;
  function f (x : integer) return integer is begin return x; end function;
begin
  process
    variable l : line;
    variable v : std_ulogic_vector(3 downto 0) := "1010";
    variable i : integer; variable a : integer_vector(0 to 3);
  begin
    write(l, string'(" v=") & hx(v));
    p(1, i + hz(2));
    one(hq(1));
    i := f(hw(1));
    i := a(hv(1));
    wait;
  end process;
end architecture;
