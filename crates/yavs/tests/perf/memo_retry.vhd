-- A pure function with a loop whose first 300 calls have distinct arguments and whose later calls
-- repeat 16: given up at its 64th call, it is taken up again later in the run and kept.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
package memo_retry_pkg is
  function f(a : unsigned(15 downto 0)) return unsigned;
end package;
package body memo_retry_pkg is
  function f(a : unsigned(15 downto 0)) return unsigned is
    variable x : unsigned(15 downto 0) := a;
  begin
    for i in 0 to 7 loop
      x := (x(14 downto 0) & x(15)) xor resize(x * 3, 16);
    end loop;
    return x;
  end;
end package body;

library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
use work.memo_retry_pkg.all;
entity memo_retry is end;
architecture a of memo_retry is
begin
  process
    variable s : natural := 0;
    variable k : natural;
  begin
    for i in 0 to 2999 loop
      if i < 300 then k := 1000 + i * 11; else k := i mod 16 * 7; end if;
      s := (s + to_integer(f(to_unsigned(k, 16)))) mod 1000003;
    end loop;
    report "s=" & integer'image(s);
    wait;
  end process;
end;
