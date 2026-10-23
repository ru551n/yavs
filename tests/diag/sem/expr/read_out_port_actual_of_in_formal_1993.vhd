-- yavs-test: std=1993
-- expect: E1360 13:39 cannot read port `o` of mode `out`
-- LRM 1993 4.3.2
entity leaf is
  port (i : in bit);
end entity;
architecture a of leaf is begin end architecture;
entity e is
  port (o : out bit);
end entity;
architecture a of e is
begin
  u : entity work.leaf port map (i => o);
end architecture;
