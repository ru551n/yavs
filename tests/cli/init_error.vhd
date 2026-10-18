package init_error_pkg is
  impure function get return integer;
end package;
package body init_error_pkg is
  type rec_t is record x : integer; end record;
  function g(x : integer) return rec_t is
  begin
    return (x => 10 / x);
  end;
  constant c : rec_t := g(0);
  impure function get return integer is begin return c.x; end;
end package body;
use work.init_error_pkg.all;
entity init_error is end;
architecture t of init_error is
begin
  process
  begin
    report integer'image(get);
    wait;
  end process;
end;
