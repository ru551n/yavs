-- difftest: top=path_name_generic_actual std=2008 ignore=order
-- Constants built from an entity's 'path_name and 'instance_name, directly and through an
-- impure function, used as generic actuals: elaboration needs their values (#314).
-- ignore=order: the leaf processes report in the same cycle.
package path_name_generic_actual_pkg is
  impure function tail(a : string) return string;
end package;

package body path_name_generic_actual_pkg is
  impure function tail(a : string) return string is
    variable r : string(1 to a'length);
  begin
    r := a;
    return r;
  end function;
end package body;

entity path_name_generic_actual_leaf is
  generic (id : string);
end entity;

architecture a of path_name_generic_actual_leaf is
begin
  process
  begin
    report id;
    wait;
  end process;
end architecture;

use work.path_name_generic_actual_pkg.all;
entity path_name_generic_actual_m is
  generic (name : string := "");
  constant n : string := tail(path_name_generic_actual_m'path_name);
  constant p : string := path_name_generic_actual_m'path_name;
end entity;

architecture a of path_name_generic_actual_m is
  constant i : string := path_name_generic_actual_m'instance_name;
begin
  l1 : entity work.path_name_generic_actual_leaf generic map (id => n & ".Rx");
  l2 : entity work.path_name_generic_actual_leaf generic map (id => p & ".Tx");
  l3 : entity work.path_name_generic_actual_leaf generic map (id => i);
end architecture;

entity path_name_generic_actual is
end entity;

architecture t of path_name_generic_actual is
begin
  m1 : entity work.path_name_generic_actual_m;
  g : for k in 0 to 1 generate
    m : entity work.path_name_generic_actual_m;
  end generate;
end architecture;
