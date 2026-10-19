-- yavs-test: std=2008
-- LRM 9.3.3.3: range choices of elements may take either direction next to one with an
-- array value; array values under choices of one direction take it without a context.
entity e is end entity;
architecture a of e is
  constant c : bit_vector := (3 downto 2 => "01", 1 downto 0 => "10");
begin
  process
    variable v : bit_vector(3 downto 0);
  begin
    v := (3 downto 2 => "01", 0 to 1 => '0');
    wait;
  end process;
end architecture;
