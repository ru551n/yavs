-- difftest: top=config_generic_types ref=hand
-- ref=hand: real'image formatting differs between the references.
-- Two bindings of one generic entity in a configuration get one instance each, with its own
-- generic type actual (#614): the second binding's ports are not checked against the first's.
entity config_generic_sub is
  generic (type t; function "+" (a, b : t) return t is <>);
  port (x, y : in t; z : out t);
end entity;
architecture test of config_generic_sub is begin z <= x + y; end architecture;

entity config_generic_r8 is end entity;
architecture test of config_generic_r8 is
  component sub_r is port (x : in real; z : out real); end component;
  component sub_i is port (x : in integer; z : out integer); end component;
  signal xr, zr : real := 1.0;
  signal xi, zi : integer := 8;
begin
  u1 : sub_r port map (xr, zr);
  u2 : sub_i port map (xi, zi);
  process begin wait for 1 ns; report real'image(zr) & " " & integer'image(zi); wait; end process;
end architecture;

configuration config_generic_types of config_generic_r8 is
  for test
    for u1 : sub_r use entity work.config_generic_sub generic map (real) port map (x => x, y => x, z => z);
    end for;
    for u2 : sub_i use entity work.config_generic_sub generic map (integer) port map (x => x, y => x, z => z);
    end for;
  end for;
end configuration;
