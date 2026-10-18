-- yavs-test: std=2008
-- expect: E1410 12:12 case choice must be locally static
-- LRM 10.9
entity e is end entity;
architecture a of e is
  signal n : integer;
begin
  process
    variable k : integer := 1;
  begin
    case n is
      when k => null;
      when others => null;
    end case;
    wait;
  end process;
end architecture;
