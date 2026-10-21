-- yavs-test: std=2008
-- expect: E1610 11:18 the aliased name is not a static name of an object
-- LRM 6.6.2, 8.1: a variable is not static, so it cannot bound the slice of an aliased name
entity e is
end entity;
architecture a of e is
  signal s : bit_vector(15 downto 0);
begin
  process
    variable v : natural := 3;
    alias bad is s(v downto 0);
  begin
    wait;
  end process;
end architecture;
