-- yavs-test: std=2008
-- expect: none
-- LRM 12.3 (an architecture named like a port: the port wins; sim1 accepts)
entity and2 is
  port (a, b : in bit; y : out bit);
end entity;
architecture a of and2 is
begin
  y <= a and b;
end architecture;
