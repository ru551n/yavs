-- difftest: top=package_subprogram_param_names std=2008 ref=hand
-- ref=hand: the reference simulator output with its library name `lib` read as `work` (the library the harness analyzes into for yavs).
-- 'instance_name and 'path_name of a parameter and of a local constant of a subprogram in a
-- library-unit package name the package, the same text from every instance that calls it (#460).
package pspn is
  procedure wr (constant idx : in integer);
end package;
package body pspn is
  procedure wr (constant idx : in integer) is
    constant c : integer := idx;
  begin
    report idx'instance_name & " | " & idx'path_name & " | " & c'instance_name;
  end procedure;
end package body;

use work.pspn.all;
entity pspn_a is end;
architecture x of pspn_a is begin
  process begin wr(1); wait; end process;
end;

use work.pspn.all;
entity pspn_b is end;
architecture y of pspn_b is begin
  process begin wr(2); wait; end process;
end;

use work.pspn.all;
entity package_subprogram_param_names is end;
architecture z of package_subprogram_param_names is begin
  i1 : entity work.pspn_a;
  i2 : entity work.pspn_b;
  i3 : entity work.pspn_a;
  process begin wait for 1 ns; wr(3); wait; end process;
end;
