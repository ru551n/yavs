-- LRM 6.5.7.1, 9.3.6: a type mark as the formal part is a type conversion of the formal:
-- INTEGER and BIT_VECTOR are not closely related.
entity sub is port (b : out integer); end entity;
architecture a of sub is begin b <= 1; end architecture;
entity top is end entity;
architecture a of top is signal t : bit_vector(3 downto 0);
begin u : entity work.sub port map (bit_vector(b) => t);
end architecture;
