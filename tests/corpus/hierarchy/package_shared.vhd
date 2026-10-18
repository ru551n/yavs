-- difftest: top=package_shared std=2008
-- A package-body shared variable of a protected type and a constant computed once from it
-- at elaboration (#183): the constant does not change between uses.
package package_shared_pb is
  impure function new_id return natural;
  impure function get_c1 return natural;
end package;
package body package_shared_pb is
  type counter_t is protected impure function next_id return natural; end protected;
  type counter_t is protected body
    variable n : natural := 0;
    impure function next_id return natural is begin n := n + 1; return n; end;
  end protected body;
  shared variable cnt : counter_t;
  impure function new_id return natural is begin return cnt.next_id; end;
  constant c1 : natural := new_id;
  impure function get_c1 return natural is begin return c1; end;
end package body;
use work.package_shared_pb.all;
entity package_shared is end;
architecture t of package_shared is begin
  process begin
    report integer'image(get_c1) & " " & integer'image(get_c1) & " " & integer'image(new_id);
    wait;
  end process;
end;
