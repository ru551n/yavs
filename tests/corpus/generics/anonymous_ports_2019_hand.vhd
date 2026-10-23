-- difftest: top=anonymous_ports_2019_hand std=2019 ref=hand
-- ref=hand: LCS-2016-016 (anonymous types in interface declarations): no reference simulator binds
-- the anonymous type of a port from its actual; the expected output is the LRM result.
-- The first entity leaves the types anonymous (taken from the actuals of the port map); the
-- second names them as generic types and gets them from a generic map.
entity e1 is
  port (
    a : in type is private;
    b : in type is <>
  );
end entity;
architecture a of e1 is
begin
  process begin report "e1"; wait; end process;
end architecture;

entity e2 is
  generic (
    type t1 is private;
    type t2 is <>
  );
  port (a : in t1; b : in t2; c : out t2);
end entity;
architecture a of e2 is
begin
  c <= b;
  process begin report "e2"; wait; end process;
end architecture;

entity anonymous_ports_2019_hand is end;
architecture t of anonymous_ports_2019_hand is
  signal sa : bit;
  signal sb : natural range 10 to 20 := 15;
  signal sc : natural range 10 to 20;
begin
  u1 : entity work.e1 port map (a => sa, b => sb);
  u2 : entity work.e2
    generic map (t1 => sa'subtype, t2 => sb'subtype)
    port map (a => sa, b => sb, c => sc);
  process begin
    wait for 1 ns;
    report integer'image(sc);
    wait;
  end process;
end architecture;
