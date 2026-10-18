-- yavs-test: std=2008
-- expect: E1405 12:12 choice has 3 elements, expected 2
-- LRM 10.9
entity e is end entity;
architecture a of e is
begin
  process
    variable s : string(1 to 2);
  begin
    case s is
      when "ab" => null;
      when "abc" => null;
      when others => null;
    end case;
    wait;
  end process;
end architecture;
