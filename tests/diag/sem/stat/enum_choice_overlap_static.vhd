-- yavs-test: std=2008
-- expect: E1820 12:12 choice 3 overlaps an earlier choice
-- LRM 10.9
entity e is end entity;
architecture a of e is
begin
  process
    variable i : integer := 0;
  begin
    case i is
      when 0 to 5 => null;
      when 3 => null;
      when others => null;
    end case;
    wait;
  end process;
end architecture;
