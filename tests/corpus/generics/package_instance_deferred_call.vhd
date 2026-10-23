-- difftest: top=package_instance_deferred_call std=2008 ref=hand
-- Deferred constants of two instances of one package declared in an architecture, computed by a
-- function of the package body and a generic. The first reference simulator stops in
-- elaboration on the function call (expected output by hand from the LRM: each instance
-- has its own values).
package pack is
  generic (g : integer);
  constant dbl : integer;
  constant s : string;
end package;

package body pack is
  function twice (x : integer) return integer is
  begin
    return 2 * x;
  end function;
  constant dbl : integer := twice(g) + 1;
  constant s : string := integer'image(g) & "!";
end package body;

entity package_instance_deferred_call is end entity;

architecture a of package_instance_deferred_call is
  package i is new work.pack generic map (5);
  package j is new work.pack generic map (7);
begin
  process
  begin
    assert i.dbl = 11 and j.dbl = 15 report "values" severity failure;
    report i.s & j.s & integer'image(i.dbl) & integer'image(j.dbl);
    wait;
  end process;
end architecture;
