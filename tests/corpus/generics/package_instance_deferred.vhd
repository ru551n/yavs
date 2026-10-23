-- difftest: top=package_instance_deferred std=2008
-- Deferred constants of a package instance declared in an architecture: read directly and
-- through a formal package of an entity, a string and an integer that folds.
package pack is
  generic (g : integer);
  constant msg : string;
  constant k : integer;
end package;

package body pack is
  constant msg : string := "hello";
  constant k : integer := g * 2;
end package body;

entity sub is
  generic (package p is new work.pack generic map (<>));
end entity;

architecture a of sub is
begin
  process
  begin
    assert p.msg = "hello" report "formal package string" severity failure;
    assert p.k = 10 report "formal package integer" severity failure;
    wait;
  end process;
end architecture;

entity package_instance_deferred is end entity;

architecture a of package_instance_deferred is
  package i is new work.pack generic map (5);
begin
  u : entity work.sub generic map (i);

  process
  begin
    assert i.msg = "hello" report "string" severity failure;
    assert i.k = 10 report "integer" severity failure;
    report "ok " & i.msg & " " & integer'image(i.k);
    wait;
  end process;
end architecture;
