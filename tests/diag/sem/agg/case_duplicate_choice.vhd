-- yavs-test: std=2008
-- expect: E1820 12:12 choice '0' is given twice
-- LRM 10.9
entity e is end entity;
architecture a of e is
begin
  process
    variable b : bit;
  begin
    case b is
      when '0' => null;
      when '0' => null;
      when '1' => null;
    end case;
    wait;
  end process;
end architecture;
