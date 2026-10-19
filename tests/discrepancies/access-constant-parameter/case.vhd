-- LRM 6.4.2.2, 6.5.2: a constant parameter of an access type.
package p is
  type int_ptr is access integer;
  procedure show(constant x : in int_ptr);
end package;
package body p is
  procedure show(constant x : in int_ptr) is begin report "RES " & integer'image(x.all); end procedure;
end package body;
use work.p.all;
entity top is end entity;
architecture a of top is
begin
  process
    variable v : int_ptr := new integer'(7);
  begin
    show(v);
    wait;
  end process;
end architecture;
