-- difftest: top=type_bounds_once std=2008
-- The constraint of a type or subtype declared in a package, an architecture, a block or a
-- generate is evaluated once, when the declaration is elaborated (LRM 14.4.2.1), used or not:
-- an impure function in a bound is called once however many objects (signals, constants,
-- variables, an elaboration-time call) use the type, and every object sees the same bounds
-- (#612). A subtype of such a subtype without a constraint of its own is the discrepancy case
-- `subtype-of-subtype-bound-once` (sim1 evaluates the bound again).
package type_bounds_once_cnt is
  type counter is protected
    procedure inc;
    impure function get return natural;
  end protected;
end package;
package body type_bounds_once_cnt is
  type counter is protected body
    variable v : natural := 0;
    procedure inc is begin v := v + 1; end procedure;
    impure function get return natural is begin return v; end function;
  end protected body;
end package body;
use work.type_bounds_once_cnt.all;
package type_bounds_once_f is
  shared variable c : counter;
  impure function f (x : integer) return integer;
  impure function g return integer;
end package;
package body type_bounds_once_f is
  impure function f (x : integer) return integer is begin c.inc; return x - 1; end function;
  impure function g return integer is begin c.inc; return c.get; end function;
end package body;
use work.type_bounds_once_f.all;
package type_bounds_once_p is
  subtype pks is bit_vector(f(4) downto 0);
  subtype pku is integer range 0 to f(4);
end package;
use work.type_bounds_once_f.all;
use work.type_bounds_once_p.all;
entity type_bounds_once is end entity;
architecture a of type_bounds_once is
  type t is array (1 to 2) of bit_vector(f(4) downto 0);
  subtype u is integer range 0 to f(4);
  subtype s is bit_vector(f(4) downto 0);
  signal sg : s;
  signal sp : pks;
  constant cc : s := (others => '0');
  subtype sv is bit_vector(g downto 1);
  function len return natural is
    variable x : s;
  begin
    return x'length;
  end function;
  constant lc : natural := len;
begin
  b : block
    subtype bs is bit_vector(f(3) downto 0);
  begin
    process
      variable v1, v2 : bs;
    begin
      report "block: " & integer'image(v1'length) & " " & integer'image(v2'length);
      wait;
    end process;
  end block;
  gen : for i in 1 to 2 generate
    subtype gs is bit_vector(f(i) downto 0);
  begin
    process
      variable v1, v2 : gs;
    begin
      report "gen " & integer'image(i) & ": " & integer'image(v1'length) & " "
        & integer'image(v2'length);
      wait;
    end process;
  end generate;
  process
    variable v1, v2 : t;
    variable w1 : s;
    variable w2 : s;
    variable x1, x2 : sv;
    variable p1, p2 : pks;
    variable n : natural;
  begin
    wait for 1 ns;
    n := c.get;
    report "calls=" & integer'image(n) & " t=" & integer'image(v1(1)'length)
      & " s=" & integer'image(w1'length) & integer'image(w2'length)
      & integer'image(sg'length) & integer'image(cc'length) & integer'image(lc)
      & " sv=" & integer'image(x1'length) & integer'image(x2'length)
      & " pks=" & integer'image(p1'length) & integer'image(sp'length)
      & " u=" & integer'image(u'high) & " pku=" & integer'image(pku'high);
    assert n = 9 report "each declaration calls f or g once" severity failure;
    assert x1'length = x2'length severity failure;
    wait;
  end process;
end architecture;
