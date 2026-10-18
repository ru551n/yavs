-- yavs-test: std=2019
-- expect: none
-- LRM 6.4.2.3, 6.4.2.4 (2019), LCS-2016-019: signals and variables take missing bounds from their initial value (#175)
entity e is
end entity;
architecture a of e is
  constant c : bit_vector(3 downto 0) := "1010";
  signal s : bit_vector := c;
begin
  process
    variable v : bit_vector := c & c;
  begin
    assert v'length = 8 and s'length = 4;
    wait;
  end process;
end architecture;
