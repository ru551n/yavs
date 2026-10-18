-- yavs-test: std=2008
-- expect: E1360 9:3 cannot assign to port `i` of mode `in`
-- LRM 6.5.6.3
entity e is
  port (i : in bit);
end entity;
architecture a of e is
begin
  i <= '0';
end architecture;
