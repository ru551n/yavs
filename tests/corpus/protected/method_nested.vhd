-- difftest: top=method_nested std=2008
-- Subprograms declared inside a method of a protected body call methods of the same body and
-- name its variables: the implicit object is the one of the enclosing method call (LRM 5.6.2).
-- Procedures and functions, nested one and two levels deep, on shared and local objects.
package method_nested_pkg is
  type prot is protected
    procedure add(i : integer);
    impure function show(i : natural) return string;
    impure function total return integer;
    impure function count return natural;
  end protected;
end package;

package body method_nested_pkg is
  type prot is protected body
    variable sum : integer := 0;
    variable cnt : natural := 0;
    variable tag : string(1 to 2) := "p:";
    procedure bump is
    begin
      cnt := cnt + 1;
    end procedure;
    impure function name(i : natural) return string is
    begin
      return tag & integer'image(i);
    end function;
    procedure add(i : integer) is
      variable twice : integer := 2 * i;
      procedure step(k : integer) is
        procedure deep(m : integer) is
        begin
          sum := sum + m;
          bump;
        end procedure;
      begin
        deep(k);
        deep(twice - k);
      end procedure;
    begin
      step(i);
    end procedure;
    impure function show(i : natural) return string is
      impure function inner(j : natural) return string is
        impure function deeper(k : natural) return string is
        begin
          bump;
          return name(k) & "/" & integer'image(sum);
        end function;
      begin
        return name(j) & " " & deeper(j + 1);
      end function;
    begin
      return inner(i);
    end function;
    impure function total return integer is begin return sum; end function;
    impure function count return natural is begin return cnt; end function;
  end protected body;
end package body;

use work.method_nested_pkg.all;
entity method_nested is end entity;
architecture a of method_nested is
  shared variable sv : prot;
begin
  process
    variable lv : prot;
  begin
    sv.add(5);
    lv.add(7);
    lv.add(1);
    assert sv.total = 10 and sv.count = 2 severity failure;
    assert lv.total = 16 and lv.count = 4 severity failure;
    report "shared " & sv.show(3) & " " & integer'image(sv.count);
    report "local " & lv.show(4) & " " & integer'image(lv.count);
    wait;
  end process;
end architecture;
