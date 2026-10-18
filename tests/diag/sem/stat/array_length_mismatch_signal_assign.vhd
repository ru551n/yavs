-- yavs-test: std=2008
-- expect: E1405 9:10 length mismatch: target has 4 elements, value has 2
-- LRM 10.5.2
entity e is end entity;
architecture a of e is
  signal s : bit_vector(3 downto 0);
begin
  process begin
    s <= "10";
    wait;
  end process;
end architecture;
