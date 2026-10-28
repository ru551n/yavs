-- yavs-test: std=2008
-- expect: E1100 14:37 entity `leaf` has no generic `dpth`
-- help: a generic with a similar name exists: `depth`
-- LRM 6.5.7.2
entity leaf is
  generic (depth : integer := 2);
end entity;
architecture rtl of leaf is
begin
end architecture;
entity top is end entity;
architecture a of top is
begin
  u : entity work.leaf generic map (dpth => 3);
end architecture;
