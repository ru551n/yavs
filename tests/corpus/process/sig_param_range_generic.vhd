-- difftest: top=sig_param_range_generic std=2008 exit=1 ref=hand
-- ref=hand: sim1 and sim2 accept the value (discrepancy `sig-param-range`). The actual's
-- range comes from a generic; a procedure of a package that waits before it drives (LRM
-- 14.7.3, #1181).
package sig_param_range_pkg is
  procedure later (signal o : inout integer; v : integer);
end package;
package body sig_param_range_pkg is
  procedure later (signal o : inout integer; v : integer) is
  begin
    wait for 1 ns;
    o <= o + v;
  end procedure;
end package body;
use work.sig_param_range_pkg.all;
entity sig_param_range_generic is
  generic (n : positive := 4);
end entity;
architecture a of sig_param_range_generic is
  signal s : integer range 0 to n := 0;
begin
  process
  begin
    later(s, 3);
    wait for 1 ns;
    report "s=" & integer'image(s);
    later(s, 2);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
