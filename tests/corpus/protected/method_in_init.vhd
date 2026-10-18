-- difftest: top=method_in_init std=2008
-- Initial values of a protected body's variables call methods of the same body: the implicit
-- object is the instance being elaborated (LRM 5.6.2), for shared and process-local variables.
package method_in_init_pkg is
  type prot is protected
    impure function get return integer;
    impure function peek return integer;
    impure function calls return natural;
    impure function name return string;
  end protected;
end package;

package body method_in_init_pkg is
  type prot is protected body
    type ptr_t is access integer;
    variable n : integer := 0;
    variable cnt : natural := 0;
    impure function init return boolean is
    begin
      n := n + 3;
      cnt := cnt + 1;
      return true;
    end function;
    impure function new_ptr return ptr_t is
    begin
      cnt := cnt + 1;
      return new integer'(n * 10);
    end function;
    constant c : boolean := init;
    variable p : ptr_t := new_ptr;
    constant tag : string := "t" & integer'image(7);
    variable m : integer := 5;
    impure function get return integer is begin return n; end function;
    impure function peek return integer is begin return p.all + m; end function;
    impure function calls return natural is begin return cnt; end function;
    impure function name return string is begin return tag; end function;
  end protected body;
end package body;

use work.method_in_init_pkg.all;
entity method_in_init is end entity;
architecture a of method_in_init is
  shared variable sv : prot;
  shared variable sv2 : prot;
begin
  process
    variable lv : prot;
  begin
    assert sv.get = 3 and sv2.get = 3 and lv.get = 3 severity failure;
    assert sv.calls = 2 and lv.calls = 2 severity failure;
    assert sv.peek = 35 and lv.peek = 35 severity failure;
    assert lv.name = "t7" severity failure;
    report "shared " & integer'image(sv.get) & " " & integer'image(sv.peek) & " " & sv.name;
    report "local " & integer'image(lv.get) & " " & integer'image(lv.peek) & " " & lv.name;
    wait;
  end process;
end architecture;
