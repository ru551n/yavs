-- yavs-test: std=2008
-- expect: none
-- LRM 10.9
entity e is end entity;
architecture a of e is
begin
  process
    variable b : boolean;
  begin
    case b is
      when true => null;
      when false => null;
    end case;
    wait;
  end process;
end architecture;
