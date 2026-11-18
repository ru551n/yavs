-- A function called while elaborating reaches a package constant without a folded value through its
-- last callee: lowering it on its own fails there, so the call runs in the session. The lowering on its
-- own stops at that first failure instead of going on with the other callees it throws away.
package lower_alone_stop_pkg is
  type iv is array (natural range <>) of integer;
  function f (i : integer) return integer;
end;
package body lower_alone_stop_pkg is
  function mk (n : integer) return iv is
    variable r : iv(0 to n - 1);
  begin
    for i in r'range loop r(i) := i * 3 + 1; end loop;
    return r;
  end;
  constant T : iv := mk(16);
  function pick (i : integer) return integer is begin return T(i); end;
  function h1 (i : integer) return integer is begin return i + 1; end;
  function h2 (i : integer) return integer is begin return h1(i) * 2; end;
  function h3 (i : integer) return integer is begin return h2(i) - 1; end;
  function h4 (i : integer) return integer is begin return h3(i) + h1(i); end;
  function h5 (i : integer) return integer is begin return h4(i) mod 7; end;
  function h6 (i : integer) return integer is begin return h5(i) + h2(i); end;
  function f (i : integer) return integer is
  begin
    return h6(i) + h5(i) + h4(i) + h3(i) + h2(i) + h1(i) + pick(i);
  end;
end;
use work.lower_alone_stop_pkg.all;
entity lower_alone_stop is end;
architecture a of lower_alone_stop is
  constant c1 : integer := f(1);
  constant c2 : integer := f(2);
  signal s1 : bit_vector(1 to c1);
  signal s2 : bit_vector(1 to c2);
begin
  process begin
    report "len=" & integer'image(s1'length + s2'length);
    wait;
  end process;
end;
