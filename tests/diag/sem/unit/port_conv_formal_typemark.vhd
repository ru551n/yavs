-- yavs-test: std=2008
-- expect: E1350 12:37 these types are not closely related
-- LRM 9.3.6
-- A type mark as the formal part converts the formal to the actual's type, so the
-- formal's type and the type mark must be closely related: INTEGER and BIT_VECTOR are not.
entity sub is
  port (b : out integer);
end entity;
architecture a of sub is begin b <= 1; end architecture;
entity top is end entity;
architecture a of top is signal t : bit_vector(3 downto 0);
begin u : entity work.sub port map (bit_vector(b) => t);
end architecture;
