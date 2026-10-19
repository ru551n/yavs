-- A TIME literal with a unit below the resolution limit (run with a nanosecond limit) in code
-- that never runs. Zero literals and a fractional value in a legal unit are fine.
package p is
  function unused return time;
end package;

package body p is
  function unused return time is
  begin
    return 1 ps;
  end function;
end package body;

use work.p.all;

entity top is end entity;

architecture a of top is
  constant zero : time := 0 ps;
  constant frac : time := 0.4 ns;
begin
  process
  begin
    report "ran";
    wait;
  end process;
end architecture;
