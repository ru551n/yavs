-- difftest: top=object_range_once_formal std=2008 exit=1 ref=hand
-- ref=hand: sim1 checks against the range of the bound's current value (discrepancy
-- `object-range-bound-variable`). The subtype of formal `x` is `0 to 9`, fixed when the
-- declaration of `put` is elaborated (LRM 14.4.2.1, #1174): after `k := 20`, a value of 10 is
-- still outside it.
entity object_range_once_formal is end entity;
architecture a of object_range_once_formal is
begin
  process
    variable k : integer := 9;
    variable q : integer := 0;
    procedure put (x : inout integer range 0 to k; v : integer) is begin x := v; end procedure;
  begin
    k := 20;
    put(q, 9);
    report "q=" & integer'image(q);
    put(q, 10);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
