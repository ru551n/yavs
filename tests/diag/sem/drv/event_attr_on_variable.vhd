-- yavs-test: std=2008
-- expect: E1621 11:12 attribute `event` requires a signal prefix, `v` is a variable
-- LRM 16.2
entity e is end entity;
architecture a of e is
begin
  process
    variable v : bit;
    variable b : boolean;
  begin
    b := v'event;
    wait;
  end process;
end architecture;
