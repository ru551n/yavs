-- LRM 6.5.7.1: individual association of an unconstrained out port leaving 5 to 8 unassociated.
entity sub is port (v : out bit_vector); end entity;
architecture a of sub is begin v <= (others => '1'); end architecture;
entity top is end entity;
architecture a of top is
  signal a, b : bit;
begin
  u : entity work.sub port map (v(4) => a, v(9) => b);
end architecture;
