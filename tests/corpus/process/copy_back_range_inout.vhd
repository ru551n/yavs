-- difftest: top=copy_back_range_inout std=2008 exit=1 ref=hand
-- ref=hand: sim1 copies the value back without a check (discrepancy `copy-back-range`); sim2
-- agrees with this output. An inout formal of an enumeration type copied back into a
-- variable of a subtype of it, from a procedure of a package (LRM 4.2.2.2, #1175).
package copy_back_range_inout_p is
  type color is (red, green, blue, white);
  subtype rgb is color range red to blue;
  procedure next_color (variable c : inout color);
end package;
package body copy_back_range_inout_p is
  procedure next_color (variable c : inout color) is
  begin
    c := color'succ(c);
  end procedure;
end package body;
use work.copy_back_range_inout_p.all;
entity copy_back_range_inout is end entity;
architecture a of copy_back_range_inout is
begin
  process
    variable q : rgb := red;
  begin
    next_color(q);
    next_color(q);
    report "q=" & rgb'image(q);
    next_color(q);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
