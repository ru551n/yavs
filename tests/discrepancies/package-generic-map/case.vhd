-- A package declaration with a generic map aspect (4.7): the generics are mapped in the
-- declaration, the package is used like an instance.
package gm is
  generic (g : integer := 1);
  generic map (g => 2);
  constant c : integer := g;
  type arr is array (0 to g) of bit;
end package;

use work.gm.all;
entity top is end;
architecture a of top is
  constant z : arr := (others => '1');
begin
  process
  begin
    report integer'image(c) & " " & integer'image(z'length);
    wait;
  end process;
end;
