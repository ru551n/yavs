-- yavs-test: std=2008
-- expect: E1100 13:10 `i` is not declared
-- LRM 12.2
entity e is end entity;
architecture a of e is
begin
  process
    variable s : integer := 0;
  begin
    for i in 0 to 3 loop
      s := s + i;
    end loop;
    s := i;
    wait;
  end process;
end architecture;
