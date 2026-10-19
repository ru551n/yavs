-- yavs-test: std=2008
-- expect: E1360 9:54 an `inertial` actual (an expression) is only allowed for an `in` port
-- LRM 6.5.6.3
entity sub is port (o : out bit_vector(1 downto 0)); end entity;
architecture r of sub is begin o <= "00"; end architecture;
entity e is end entity;
architecture a of e is
  signal s, t : bit;
begin u : entity work.sub port map (o(1) => inertial s, o(0) => t);
end architecture;
