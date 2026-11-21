-- difftest: top=sig_param_range_kinds std=2008 exit=1 ref=hand
-- ref=hand: sim1 and sim2 accept the values (discrepancy `sig-param-range`). Enumeration,
-- physical and floating-point actuals narrower than the formal: values in range go through,
-- 'Z' does not fit `X01` (LRM 14.7.3, #1181).
library ieee;
use ieee.std_logic_1164.all;
entity sig_param_range_kinds is end entity;
architecture a of sig_param_range_kinds is
  subtype short is time range 0 ns to 5 ns;
  subtype unit is real range 0.0 to 1.0;
  signal x : X01 := '0';
  signal t : short := 0 ns;
  signal u : unit := 0.0;
  procedure dl (signal o : out std_ulogic; v : std_ulogic) is begin o <= v; end procedure;
  procedure dt (signal o : out time; v : time) is begin o <= v; end procedure;
  procedure du (signal o : out real; v : real) is begin o <= v; end procedure;
begin
  process
  begin
    dl(x, '1');
    dt(t, 4 ns);
    du(u, 0.5);
    wait for 1 ns;
    report "x=" & std_ulogic'image(x) & " t=" & time'image(t) & " u2=" & integer'image(integer(u * 2.0));
    dl(x, 'Z');
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
