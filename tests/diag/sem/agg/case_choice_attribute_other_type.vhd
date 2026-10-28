-- yavs-test: std=2008
-- expect: E1300 12:12 expected `INTEGER`, found `small`
-- LRM 10.9: each choice is of the type of the case expression (#822)
entity e is end;
architecture a of e is
  type small is range 0 to 100;
begin
  process
    variable i : integer := 0;
  begin
    case i is
      when small'low to 60 => null;
      when others => null;
    end case;
    wait;
  end process;
end;
