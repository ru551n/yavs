-- yavs-test: std=2008
-- expect: E1326 11:11 a non-static choice must be the only choice of the only association
-- LRM 9.3.3.3
entity e is end entity;
architecture a of e is
begin
  process
    variable i : integer := 1;
    variable v : bit_vector(0 to 3);
  begin
    v := (i => '1', others => '0');
    wait;
  end process;
end architecture;
