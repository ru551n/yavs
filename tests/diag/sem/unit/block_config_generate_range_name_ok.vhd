-- yavs-test: std=2008
-- expect: none
-- LRM 1.3.1, 3.4.2 (the index specification of a block configuration is a discrete range:
-- a range attribute or a subtype name; #646)
entity c is generic (n : integer := 0); end entity;
architecture rtl of c is begin end architecture;
entity top is end entity;
architecture a of top is
  component c is generic (n : integer := 0); end component;
  subtype idx is integer range 1 to 2;
  type vec is array (0 to 3) of bit;
begin
  g : for i in vec'range generate
    u : c;
  end generate;
  h : for i in idx generate
    v : c;
  end generate;
end architecture;
configuration cfg of top is
  for a
    for g(vec'range)
      for u : c use entity work.c(rtl); end for;
    end for;
    for g(vec'reverse_range)
      for u : c use entity work.c(rtl); end for;
    end for;
    for h(idx)
      for v : c use entity work.c(rtl); end for;
    end for;
  end for;
end configuration;
