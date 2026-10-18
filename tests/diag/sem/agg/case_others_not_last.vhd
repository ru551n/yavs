-- yavs-test: std=2008
-- expect: E1820 11:12 `others` must be the last alternative
-- LRM 10.9
entity e is end entity;
architecture a of e is
begin
  process
    variable i : integer;
  begin
    case i is
      when others => null;
      when 1 => null;
    end case;
    wait;
  end process;
end architecture;
