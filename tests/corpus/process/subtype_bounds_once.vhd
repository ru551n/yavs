-- difftest: top=subtype_bounds_once std=2008
-- The constraint of a type or subtype declared in a process or subprogram is evaluated once,
-- when the declaration is elaborated (LRM 14.4.2.1): a variable the bound reads may change
-- later without changing the subtype, and an impure function in the bound is called once
-- however many objects use the type (#612). A scalar subtype is the discrepancy case
-- `subtype-range-bound-variable` (sim1 reads the variable again).
package subtype_bounds_once_cnt is
  type counter is protected
    procedure inc;
    impure function get return natural;
  end protected;
end package;
package body subtype_bounds_once_cnt is
  type counter is protected body
    variable v : natural := 0;
    procedure inc is begin v := v + 1; end procedure;
    impure function get return natural is begin return v; end function;
  end protected body;
end package body;
use work.subtype_bounds_once_cnt.all;
package subtype_bounds_once_f is
  shared variable c : counter;
  impure function f (x : integer) return integer;
end package;
package body subtype_bounds_once_f is
  impure function f (x : integer) return integer is begin c.inc; return x - 1; end function;
end package body;
use work.subtype_bounds_once_f.all;
entity subtype_bounds_once is end entity;
architecture a of subtype_bounds_once is
  procedure p is
    variable n : integer := 3;
    subtype st is bit_vector(n downto 0);
    type at is array (1 to 2) of bit_vector(n downto 0);
    procedure q is
      variable x : st;
      variable y : at;
    begin
      report "q: x'length=" & integer'image(x'length) & " y(1)'length="
        & integer'image(y(1)'length);
    end procedure;
    variable v : st;
  begin
    n := 7;
    report "p: st'length=" & integer'image(st'length) & " v'length=" & integer'image(v'length);
    q;
  end procedure;
  procedure r is
    type t is array (1 to 2) of bit_vector(f(4) downto 0);
    subtype s is bit_vector(f(4) downto 0);
    variable v1, v2 : t;
    variable w1, w2 : s;
  begin
    report "r: v1(1)'length=" & integer'image(v1(1)'length) & " w2'length="
      & integer'image(w2'length);
  end procedure;
begin
  process
    variable n : integer := 2;
    subtype pst is bit_vector(n downto 0);
    type pt is array (1 to 2) of bit_vector(f(4) downto 0);
    type unused is array (1 to f(5)) of bit;
    variable a1, a2 : pt;
    variable before : natural;
  begin
    n := 9;
    report "process: pst'length=" & integer'image(pst'length) & " calls=" & integer'image(c.get);
    p;
    before := c.get;
    r;
    report "r calls=" & integer'image(c.get - before);
    wait;
  end process;
end architecture;
