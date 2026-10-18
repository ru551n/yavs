-- yavs-test: std=2008
-- expect: E1410 17:26 formal index is a signal, not locally static
-- LRM 6.5.7.1
-- The formal designator of a sub-element association is a locally static name.
entity c is
  port (x : in bit_vector(1 downto 0));
end entity;
architecture a of c is
begin
end architecture;
entity e is end entity;
architecture a of e is
  signal i : natural := 0;
  signal s : bit;
begin
  u : entity work.c
    port map (x(1) => s, x(i) => s);
end architecture;
