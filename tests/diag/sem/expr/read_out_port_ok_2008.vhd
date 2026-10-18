-- yavs-test: std=2008
-- expect: none
-- LRM 6.5.6.3
entity e is
  port (o : out bit);
end entity;
architecture a of e is
  signal s : bit;
begin
  s <= o;
end architecture;
