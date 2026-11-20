-- difftest: top=object_range_once_error std=2008 exit=1 ref=hand
-- ref=hand: sim1 checks against the range of the bound's current value (discrepancy
-- `object-range-bound-variable`). The anonymous subtype of `q` is `0 to 9`, fixed at its
-- declaration (LRM 14.4.2.1, #1174): after `k := 20`, a value of 10 is still outside it
-- when assigned.
entity object_range_once_error is end entity;
architecture a of object_range_once_error is
  procedure set10 (variable o : out integer) is begin o := 10; end procedure;
begin
  process
    variable k : integer := 9;
    variable q : integer range 0 to k := 0;
  begin
    k := 20;
    q := 9;
    report "q=" & integer'image(q);
    q := 10;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
