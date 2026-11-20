-- difftest: top=state_range_once_error std=2008 exit=1 ref=hand
-- ref=hand: sim1 checks against the range of the bound's current value (discrepancy
-- `object-range-bound-variable`); sim2 agrees with this output. The subtype of the object's
-- `q` is `0 to 9`, fixed when the object is created (LRM 14.4.2.1, #1176): after `k := 20`,
-- a value of 10 is still outside it.
entity state_range_once_error is end entity;
architecture a of state_range_once_error is
  type pt is protected
    procedure set (v : integer);
    impure function get return integer;
  end protected;
  type pt is protected body
    variable k : integer := 9;
    variable q : integer range 0 to k := 0;
    procedure set (v : integer) is begin k := 20; q := v; end procedure;
    impure function get return integer is begin return q; end function;
  end protected body;
  shared variable sp : pt;
begin
  process begin
    sp.set(9);
    report "q=" & integer'image(sp.get);
    sp.set(10);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
