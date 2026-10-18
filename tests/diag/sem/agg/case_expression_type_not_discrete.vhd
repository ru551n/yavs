-- yavs-test: std=2008
-- expect: E1300 10:10 case expression must be of a discrete or 1-D character array type, found `real`
-- LRM 10.9
entity e is end entity;
architecture a of e is
begin
  process
    variable r : real;
  begin
    case r is
      when others => null;
    end case;
    wait;
  end process;
end architecture;
