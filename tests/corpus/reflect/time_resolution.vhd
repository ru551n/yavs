-- difftest: top=time_resolution std=2019 res=ps ref=hand
-- ref=hand: TIME mirror values are in resolution-limit units (docs/decisions.md "Reflection:
-- TIME values in mirrors"); the reference simulator always reports femtoseconds. Run at
-- --resolution=ps: 5 ns is 5000 of unit 2 (PS), the unit equal to the resolution limit.
use std.reflection.all;
use std.env.all;
entity time_resolution is end entity;

architecture test of time_resolution is
begin
  process
    variable t : time := 5 ns;
    variable pv : physical_value_mirror;
  begin
    pv := t'reflect.to_physical;
    report "limit " & integer'image(resolution_limit / 1 ps) & " ps";
    report "t value " & integer'image(pv.value) & " unit " & index'image(pv.unit_index);
    wait;
  end process;
end architecture;
