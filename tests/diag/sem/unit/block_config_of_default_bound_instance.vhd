-- yavs-test: std=2008
-- expect: E1704 15:7 component `c` is only default-bound, so a block configuration cannot configure its architecture
-- LRM 3.4.3, 7.3.3 (a default binding is not enough: two of the three reference simulators reject it)
entity c is generic (n : integer := 0); end entity;
architecture rtl of c is begin end architecture;
entity top is end entity;
architecture a of top is
  component c is generic (n : integer := 0); end component;
begin
  u : c;
end architecture;
configuration cfg of top is
  for a
    for u : c
      for rtl end for;
    end for;
  end for;
end configuration;
