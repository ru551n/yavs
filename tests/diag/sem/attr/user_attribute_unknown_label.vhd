-- yavs-test: std=2008
-- expect: E1100 7:19 `ghost` is not declared
-- LRM 7.2
entity e is end entity;
architecture a of e is
  attribute kh : string;
  attribute kh of ghost : label is "TRUE";
  signal s : bit;
begin
  u1 : s <= '1';
end architecture;
