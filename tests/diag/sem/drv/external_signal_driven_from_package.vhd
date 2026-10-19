-- yavs-test: std=2008
-- expect: E1810 11:5 an external signal is driven from a subprogram declared outside a process
-- LRM 10.5.2.1: a procedure outside a process drives only its signal parameters; an
-- external name is no exception (force and release are allowed, #255).
package p is
  procedure poke;
end package;
package body p is
  procedure poke is
  begin
    <<signal .tb.dut.s : bit>> <= '1';
    <<signal .tb.dut.s : bit>> <= force '1';
  end procedure;
end package body;
