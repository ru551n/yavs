-- yavs-test: std=1993
-- expect: E1360 10:8 cannot read port `o` of mode `out`
-- LRM 1993 4.3.2
entity e is
  port (o : out bit);
end entity;
architecture a of e is
  signal b : boolean;
begin
  b <= o'event;
end architecture;
