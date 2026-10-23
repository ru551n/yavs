-- difftest: top=package_generic_map std=2008
-- A package declaration with a generic clause and a generic map aspect (4.7) in a library: it is used
-- like an instance (#598).
package gm_pkg is
  generic (g : integer := 1; k : integer := 10);
  generic map (g => 2);
  constant c : integer := g + k;
  type arr is array (0 to g) of bit;
  type rec is record a : arr; end record;
  constant r : rec := (a => (others => '1'));
  signal cnt : integer := g;
end package;

use work.gm_pkg.all;
entity package_generic_map is end;
architecture t of package_generic_map is
  constant z : arr := (others => '1');
begin
  process
  begin
    cnt <= cnt + 5;
    wait for 1 ns;
    assert c = 12 and z'length = 3 and r.a'length = 3 and cnt = 7 severity failure;
    report integer'image(c) & " " & integer'image(z'length) & " " & integer'image(cnt);
    wait;
  end process;
end architecture;
