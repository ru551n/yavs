-- yavs-test: std=2008
-- expect: E1704 9:15 a configuration specification needs an entity aspect
-- LRM 7.3.2.1
entity c is generic (n : integer := 0); end entity;
architecture rtl of c is begin end architecture;
entity top is end entity;
architecture a of top is
  component c is generic (n : integer := 0); end component;
  for all : c generic map (n => 1);
begin
  u : c;
end architecture;
