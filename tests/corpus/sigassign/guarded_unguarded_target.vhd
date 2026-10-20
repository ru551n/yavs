-- difftest: top=guarded_unguarded_target
-- A guarded concurrent assignment assigns only while the guard condition holds, also in the
-- delta where the condition's signal changes (LRM 11.6); the target is unguarded (#373).
library ieee;
use ieee.std_logic_1164.all;
entity guarded_unguarded_target is end;
architecture a of guarded_unguarded_target is
  signal v : natural := 0;
  signal o, p : natural;
  signal bs : std_logic bus;
  signal sel : natural := 0;
  disconnect bs : std_logic after 0 ns;
begin
  b : block (v < 10) is
  begin
    o <= guarded v * 2;
    bs <= guarded '1';
    with sel select p <= guarded v + 100 when 0, v + 200 when others;
  end block;
  process
  begin
    v <= 3;
    wait for 1 ns;
    v <= 10;
    wait for 1 ns;
    assert o = 6 and p = 103 report "o = " & integer'image(o) & " p = " & integer'image(p) severity failure;
    sel <= 1;
    wait for 1 ns;
    assert bs = 'Z' report "bs = " & std_logic'image(bs) severity failure;
    assert p = 103 report "p = " & integer'image(p) severity failure;
    v <= 4;
    wait for 1 ns;
    report "o = " & integer'image(o) & " p = " & integer'image(p);
    wait;
  end process;
end;
