-- yavs-test: std=2008
-- expect: E1361 11:38 a subtype indication is not an expression
-- expect: E1361 12:38 a range is not an expression
-- LRM 6.5.7.1, 6.5.7.2: only a generic type takes a subtype indication, no formal a range (#35)
entity sub is port (p : in bit_vector(3 downto 0)); end entity;
architecture a of sub is begin end architecture;
entity t2 is end entity;
architecture a of t2 is
  signal s : bit_vector(7 downto 0);
begin
  u : entity work.sub port map (p => integer range 1 to 3);
  v : entity work.sub port map (p => 3 downto 0);
end architecture;
