-- Locals of arrays of vectors initialised with `others` aggregates, in functions called in a
-- loop: with static bounds the variable is one copy of the constant, with run-time bounds one
-- fill of all its planes, then the object made from it; no row copied by a loop, no temporary
-- copied twice.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity agg_locals is end;
architecture t of agg_locals is
  type t_arr is array (natural range <>) of signed(15 downto 0);
  function f_static (x : integer) return integer is
    variable r : t_arr(0 to 7) := (others => (others => '0'));
  begin
    r(x mod 8) := to_signed(x, 16);
    return to_integer(r(3));
  end function;
  function f_dyn (x : t_arr) return integer is
    variable r : t_arr(x'range) := (others => (others => '0'));
  begin
    r(x'low) := x(x'low);
    return to_integer(r(x'low));
  end function;
begin
  process
    variable acc : integer := 0;
    constant sa : t_arr(0 to 7) := (others => to_signed(1, 16));
  begin
    for i in 1 to 40 loop
      acc := acc + f_static(i) + f_dyn(sa);
    end loop;
    report "acc=" & integer'image(acc);
    wait;
  end process;
end;
