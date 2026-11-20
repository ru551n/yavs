-- difftest: top=state_range_once std=2008 ref=hand
-- ref=hand: sim1 reads the bound again at each check (discrepancy `object-range-bound-variable`);
-- sim2 agrees with this output.
-- The range of a protected state variable's anonymous subtype is evaluated once per object,
-- when the object is created and the variable's declaration is elaborated (LRM 14.4.2.1):
-- a bound may read an earlier state variable, which may change later without changing the
-- variable's subtype, and an impure function in a bound is called once per object (#1176).
package state_range_once_c is
  type counter is protected
    impure function bump (by : integer) return integer;
  end protected;
end package;
package body state_range_once_c is
  type counter is protected body
    variable v : integer := 0;
    impure function bump (by : integer) return integer is
    begin
      v := v + by;
      return v;
    end function;
  end protected body;
end package body;
use work.state_range_once_c.all;
package state_range_once_p is
  shared variable c : counter;
  shared variable ks : counter;
  impure function counted (x : integer) return integer;
  impure function calls return integer;
  impure function next_k return integer;
end package;
package body state_range_once_p is
  impure function counted (x : integer) return integer is
    variable n : integer;
  begin
    n := c.bump(1);
    return x;
  end function;
  impure function calls return integer is begin return c.bump(0); end function;
  -- 9, 7, 5, ...
  impure function next_k return integer is begin return 11 - ks.bump(2); end function;
end package body;
use work.state_range_once_p.all;
entity state_range_once is end entity;
architecture a of state_range_once is
  type pt is protected
    procedure set (v : integer);
    impure function get return integer;
    impure function hi return integer;
    impure function down return integer;
    impure function w_of return integer;
    procedure fresh (v : integer);
  end protected;
  type pt is protected body
    variable k : integer := next_k;
    variable q : integer range 0 to k := 0;
    -- No initial value: the default is the left bound, the object's `k`.
    variable d : integer range k downto 2;
    variable w : integer range 0 to counted(20) := 1;
    procedure set (v : integer) is
      -- A subprogram nested in a method sees the object's bounds too.
      procedure put (x : integer) is begin q := x; end procedure;
    begin
      k := 1;
      put(v);
      w := v; w := 0; w := v;
    end procedure;
    impure function get return integer is begin return q; end function;
    impure function hi return integer is begin return q'subtype'high; end function;
    impure function down return integer is begin return d; end function;
    impure function w_of return integer is begin return w; end function;
    -- A new object of the same type here has its own bounds (`0 to 3`); `q` is `self`'s.
    procedure fresh (v : integer) is
      variable other : pt;
    begin
      q := v;
    end procedure;
  end protected body;
  shared variable a9 : pt;
  shared variable b7 : pt;
begin
  process
    -- Created after the methods are lowered for the shared variables.
    variable l : pt;
  begin
    report "calls=" & integer'image(calls);
    assert calls = 3 severity failure;
    a9.set(5);
    report "a q=" & integer'image(a9.get) & " high=" & integer'image(a9.hi)
      & " d=" & integer'image(a9.down) & " w=" & integer'image(a9.w_of);
    a9.set(9);
    b7.set(4);
    l.set(3);
    report "b q=" & integer'image(b7.get) & " high=" & integer'image(b7.hi)
      & " d=" & integer'image(b7.down);
    report "l q=" & integer'image(l.get) & " high=" & integer'image(l.hi)
      & " d=" & integer'image(l.down);
    assert a9.hi = 9 and b7.hi = 7 and l.hi = 5 severity failure;
    report "calls=" & integer'image(calls);
    assert calls = 3 severity failure;
    a9.fresh(9);
    report "fresh q=" & integer'image(a9.get) & " calls=" & integer'image(calls);
    wait;
  end process;
end architecture;
