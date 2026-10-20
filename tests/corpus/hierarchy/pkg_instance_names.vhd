-- difftest: top=pkg_instance_names std=2008 ref=hand
-- ref=hand: LRM 16.2.5 names an item of a package or package instance by that package
-- (`:lib:pkg:` for a library unit, else the declaring region plus the instance). The
-- reference simulators each name some of these cases after the caller's context instead.
package g is
  generic (k : integer);
  constant c : integer := k;
  function nm return string;
  function pn return string;
end package;
package body g is
  function nm return string is begin return nm'instance_name; end;
  function pn return string is begin return pn'path_name; end;
end package body;
package lib_inst is new work.g generic map (k => 1);
package plain is
  function nm return string;
end package;
package body plain is
  function nm return string is begin return nm'instance_name & "|" & nm'path_name; end;
end package body;
entity pkg_instance_names is end;
architecture t of pkg_instance_names is
  package local is new work.g generic map (k => 3);
begin
  process
  begin
    report "lib nm=" & work.lib_inst.nm;
    report "lib pn=" & work.lib_inst.pn;
    report "local nm=" & local.nm;
    report "local pn=" & local.pn;
    report "local c=" & local.c'instance_name & " | " & local.c'path_name;
    report "plain=" & work.plain.nm;
    wait;
  end process;
end;
