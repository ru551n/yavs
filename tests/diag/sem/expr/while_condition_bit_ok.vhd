-- yavs-test: std=2008
-- expect: none
-- LRM 9.2.9
entity e is end entity;
architecture a of e is
begin
  process
    variable b : bit;
  begin
    while b loop
      null;
    end loop;
    wait;
  end process;
end architecture;
