-- yavs-test: std=2008
-- expect: E1311 13:42 gap before p(2); note names index 1
-- LRM 6.5.7.1
-- Port maps follow the same rule: p(0) and p(2) leave p(1) without an actual.
entity c is
  port (p : out bit_vector);
end entity;
architecture a of c is begin end architecture;
entity e is end entity;
architecture a of e is
  signal x, y : bit;
begin
  u : entity work.c port map (p(0) => x, p(2) => y);
end architecture;
