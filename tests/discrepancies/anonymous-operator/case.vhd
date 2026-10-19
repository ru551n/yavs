-- A 2019 user operator whose parameters have anonymous types (`type is ...`), called with operator syntax.
package p is
  function "<" (a : type is private; b : a'subtype) return boolean;
end package;
package body p is
  function "<" (a : type is private; b : a'subtype) return boolean is begin return true; end function;
end package body;
use work.p.all;
entity top is end;
architecture a of top is
  type rec is record x : integer; end record;
begin
  process
    variable r : rec;
  begin
    report "RES " & boolean'image(r < r);
    wait;
  end process;
end architecture;
