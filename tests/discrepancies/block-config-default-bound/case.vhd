-- yavs-test: std=2008
-- expect: none
-- LRM 3.4.3, 7.3.3 (the LRM does not forbid it: accepted, user 2026-10-19, rule 15)
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
