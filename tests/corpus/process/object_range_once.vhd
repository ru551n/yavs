-- difftest: top=object_range_once std=2008 ref=hand
-- ref=hand: sim1 evaluates the range of an object's anonymous subtype again at every check
-- (discrepancy `object-range-bound-variable`); sim2 agrees with this output. The range of the
-- anonymous subtype of a variable, constant or formal is evaluated once, when its declaration
-- is elaborated (LRM 14.4.2.1): a variable the bound reads may change later without changing
-- the object's subtype, and an impure function in the bound is called once (#1174).
package object_range_once_cnt is
  type counter is protected
    procedure inc;
    impure function get return natural;
  end protected;
end package;
package body object_range_once_cnt is
  type counter is protected body
    variable v : natural := 0;
    procedure inc is begin v := v + 1; end procedure;
    impure function get return natural is begin return v; end function;
  end protected body;
end package body;
use work.object_range_once_cnt.all;
package object_range_once_f is
  shared variable c : counter;
  impure function f (x : integer) return integer;
end package;
package body object_range_once_f is
  impure function f (x : integer) return integer is begin c.inc; return x; end function;
end package body;
use work.object_range_once_f.all;
entity object_range_once is end entity;
architecture a of object_range_once is
  type ivec is array (natural range <>) of integer;
  -- A subprogram's variables: the bound reads a parameter and a variable of the subprogram.
  procedure sub (n : integer; r : out integer) is
    variable k : integer := n;
    variable q : integer range 0 to k := 0;
    variable w : integer range 0 to f(n) := 0;
    variable bv : bit_vector(0 to k);
  begin
    k := 1;
    q := n;
    w := n; w := 0; w := n;
    bv := (others => '1');
    r := q + bv'length;
  end procedure;
  procedure set8 (variable o : out integer) is begin o := 8; end procedure;
begin
  process
    variable k : integer := 9;
    variable q : integer range 0 to k := 0;
    constant cq : integer range 0 to k := 3;
    variable bv : bit_vector(0 to k) := (others => '0');
    variable iv : ivec(1 to k);
    variable r : integer range 0 to f(5) := 0;
    variable s : integer;
    -- A formal's subtype is elaborated with the subprogram declaration.
    procedure put (x : inout integer range 0 to k) is begin x := 7; end procedure;
  begin
    report "decl calls=" & integer'image(c.get);
    assert c.get = 1 severity failure;
    k := 1;
    q := 5;
    report "q=" & integer'image(q) & " cq=" & integer'image(cq);
    set8(q);
    report "copy back q=" & integer'image(q);
    put(q);
    report "formal q=" & integer'image(q);
    bv := (others => '1');
    iv := (others => 3);
    report "bv'length=" & integer'image(bv'length) & " iv'high=" & integer'image(iv'high);
    r := 1; r := 2; r := 3;
    report "uses calls=" & integer'image(c.get);
    assert c.get = 1 severity failure;
    sub(6, s);
    report "sub r=" & integer'image(s) & " calls=" & integer'image(c.get);
    assert s = 13 and c.get = 2 severity failure;
    sub(3, s);
    report "sub r=" & integer'image(s) & " calls=" & integer'image(c.get);
    assert s = 7 and c.get = 3 severity failure;
    wait;
  end process;
end architecture;
