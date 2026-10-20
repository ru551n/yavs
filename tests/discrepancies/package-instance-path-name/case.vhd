-- 'instance_name and 'path_name of a function and a constant of a package instance declared
-- in an architecture, read through the instance (LRM 16.2.5 names an item of a package
-- instance by the instance: the architecture path, then the instance label).
package g is
  generic (k : integer);
  constant c : integer := k;
  function nm return string;
end package;
package body g is
  function nm return string is
  begin
    return nm'instance_name;
  end function;
end package body;

entity pkginstname is end entity;

architecture test of pkginstname is
  package local is new work.g generic map (k => 3);
begin
  process
  begin
    report "RES function " & local.nm;
    report "RES constant " & local.c'path_name;
    wait;
  end process;
end architecture;
