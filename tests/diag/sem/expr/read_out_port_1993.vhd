-- yavs-test: std=1993
-- expect: E1360 11:8 cannot read port `o` of mode `out`
-- help: declare it `inout` or `buffer`
-- LRM 1993 4.3.2
entity e is
  port (o : out bit);
end entity;
architecture a of e is
  signal s : bit;
begin
  s <= o;
end architecture;
