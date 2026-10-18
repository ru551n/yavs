-- yavs-test: std=2008
-- expect: E1820 10:5 case does not cover value '1'
-- LRM 10.9
entity e is end entity;
architecture a of e is
begin
  process
    variable b : bit;
  begin
    case b is
      when '0' => null;
    end case;
    wait;
  end process;
end architecture;
