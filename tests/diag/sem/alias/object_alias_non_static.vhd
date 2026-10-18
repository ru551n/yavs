-- yavs-test: std=2008
-- expect: E1610 10:16 the aliased name is not a static name of an object
-- LRM 6.6.2
entity e is end entity;
architecture a of e is
  signal s : bit_vector(0 to 7);
begin
  process
    variable i : integer := 0;
    alias x is s(i);
  begin
    wait;
  end process;
end architecture;
