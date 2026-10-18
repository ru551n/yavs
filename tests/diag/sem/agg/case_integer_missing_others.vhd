-- yavs-test: std=2008
-- expect: E1820 10:5 case does not cover value 3 and has no `others`
-- LRM 10.9
entity e is end entity;
architecture a of e is
begin
  process
    variable i : integer range 0 to 3;
  begin
    case i is
      when 0 to 2 => null;
    end case;
    wait;
  end process;
end architecture;
