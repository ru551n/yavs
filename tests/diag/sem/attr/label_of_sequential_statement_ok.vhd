-- yavs-test: std=2008
-- expect: none
-- LRM 7.2 (a process or subprogram declarative part decorates the label of a statement of its body; #645)
entity e is end entity;
architecture a of e is
  attribute hint : string;
  function f return integer is
    attribute hint of lp : label is "function";
    variable n : integer := 0;
  begin
    lp : for i in 1 to 3 loop n := n + i; end loop;
    return n;
  end function;
begin
  controller : process is
    attribute hint of control_loop : label is "FSM";
    attribute hint of inner : label is "nested";
  begin
    control_loop : loop
      inner : for i in 1 to 2 loop null; end loop;
      wait;
    end loop;
  end process;
end architecture;
