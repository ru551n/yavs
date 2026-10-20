-- difftest: top=conf_ban std=2008
-- The actuals of a binding indication see the names visible where the binding is written: a
-- constant of the architecture (configuration specification), an enumeration literal made
-- visible by the context clause of a configuration declaration.
package pk_ban is
  type color is (red, green, blue);
  component cell is
    generic (n : integer := 1; c : color := red);
  end component;
end package;

use work.pk_ban.all;
entity sub_ban is
  generic (n : integer := 2; c : color := red);
end entity;
architecture test of sub_ban is
begin
  process begin
    report "n=" & integer'image(n) & " c=" & color'image(c);
    wait;
  end process;
end architecture;

use work.pk_ban.all;
entity tb_binding_actual_names is end entity;
architecture arch of tb_binding_actual_names is
  constant c7 : integer := 7;
  for u1 : cell use entity work.sub_ban(test) generic map (n => c7);
begin
  u1 : component cell;
  u2 : component cell;
end architecture;

use work.pk_ban.all;
configuration conf_ban of tb_binding_actual_names is
  for arch
    for u2 : cell
      use entity work.sub_ban(test) generic map (n => 3, c => blue);
    end for;
  end for;
end configuration;
