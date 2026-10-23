-- yavs-test: std=2008
-- expect: E1100 9:23 `nowhere` is not declared
-- LRM 7.2 (the label names no statement of the process)
entity e is end entity;
architecture a of e is
  attribute hint : string;
begin
  controller : process is
    attribute hint of nowhere : label is "FSM";
  begin
    wait;
  end process;
end architecture;
