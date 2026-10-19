-- difftest: top=pkg_generic_kinds std=2008
-- Package generics of every kind: constant, type, subprogram and package.
package base_g is
  generic (n : integer; type t; function conv(x : t) return integer);
  constant size : integer := n * 2;
  function sum(a, b : t) return integer;
end package;
package body base_g is
  function sum(a, b : t) return integer is
  begin
    return conv(a) + conv(b) + n;
  end function;
end package body;

package user_g is
  generic (package b is new work.base_g generic map (<>));
  use b.all;
  function total(x : b.t) return integer;
  constant user_size : integer := b.size + 1;
end package;
package body user_g is
  function total(x : b.t) return integer is
  begin
    return b.sum(x, x) * b.n;
  end function;
end package body;

package helpers is
  function bool_conv(x : boolean) return integer;
  function char_conv(x : character) return integer;
end package;
package body helpers is
  function bool_conv(x : boolean) return integer is
  begin
    if x then return 1; else return 0; end if;
  end function;
  function char_conv(x : character) return integer is
  begin
    return character'pos(x);
  end function;
end package body;

use work.helpers.all;
package base_bool is new work.base_g generic map (n => 3, t => boolean, conv => bool_conv);
use work.helpers.all;
package base_char is new work.base_g generic map (n => 10, t => character, conv => char_conv);
package user_bool is new work.user_g generic map (b => work.base_bool);
package user_char is new work.user_g generic map (b => work.base_char);

entity pkg_generic_kinds is end entity;
architecture t of pkg_generic_kinds is
begin
  process
  begin
    report "bool size=" & integer'image(work.base_bool.size)
      & " sum=" & integer'image(work.base_bool.sum(true, false));
    report "char size=" & integer'image(work.base_char.size)
      & " sum=" & integer'image(work.base_char.sum('A', 'B'));
    report "user_bool total=" & integer'image(work.user_bool.total(true))
      & " size=" & integer'image(work.user_bool.user_size);
    report "user_char total=" & integer'image(work.user_char.total('a'))
      & " size=" & integer'image(work.user_char.user_size);
    assert work.user_char.total('a') = (97 + 97 + 10) * 10 severity failure;
    wait;
  end process;
end architecture;
