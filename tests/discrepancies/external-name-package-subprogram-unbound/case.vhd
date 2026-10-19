-- LRM 8.7: an external name in a package subprogram is evaluated when the subprogram runs;
-- one that names nothing is no error while the subprogram is not called.
package p is
  impure function never return integer;
end package;
package body p is
  impure function never return integer is
  begin
    return <<constant .top.nothing.here : integer>>;
  end function;
end package body;
use work.p.all;
entity top is end;
architecture t of top is
begin
  process
  begin
    report "RES ok";
    wait;
  end process;
end;
