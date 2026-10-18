-- yavs-test: std=2008
-- expect: none
-- LRM 3.4.2 (generate_specification may be a static discrete range)
entity c is generic (n : integer := 0); end entity;
architecture rtl of c is begin end architecture;
entity top is end entity;
architecture a of top is
  component c is generic (n : integer := 0); end component;
begin
  g : for i in 0 to 3 generate
    u : c;
  end generate;
end architecture;
configuration cfg of top is
  for a
    for g(1 to 2)
      for u : c use entity work.c(rtl); end for;
    end for;
  end for;
end configuration;
