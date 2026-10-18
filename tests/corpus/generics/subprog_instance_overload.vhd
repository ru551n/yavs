-- difftest: top=subprog_instance_overload
-- Several instances of one generic function declared under the generic's own name
-- (`function f is new f generic map (...)`) overload by their instantiated profiles
-- (LRM 4.4, 12.3), next to an ordinary function of the same name, in a package and in an
-- architecture that also sees the package's instances (#28).
package sio_gen_pkg is
  function describe generic (type t; constant tag : string) parameter (v : t) return string;
end package;
package body sio_gen_pkg is
  function describe generic (type t; constant tag : string) parameter (v : t) return string is
  begin
    return tag;
  end;
end package body;

use work.sio_gen_pkg.all;
package sio_inst_pkg is
  type point_t is record x, y : integer; end record;
  type color_t is (red, green);
  function describe is new describe generic map (t => point_t, tag => "point");
  function describe is new describe generic map (t => integer, tag => "integer");
  function describe(v : real) return string;
end package;
package body sio_inst_pkg is
  function describe(v : real) return string is
  begin
    return "real";
  end;
end package body;

use work.sio_gen_pkg.all;
use work.sio_inst_pkg.all;
entity subprog_instance_overload is end entity;
architecture a of subprog_instance_overload is
  type flags_t is record up, down : boolean; end record;
  function describe is new describe generic map (t => flags_t, tag => "flags");
  function describe is new describe generic map (t => color_t, tag => "color");
begin
  process
    variable p : point_t := (1, 2);
    variable f : flags_t := (true, false);
  begin
    assert describe(p) = "point" severity failure;
    assert describe(17) = "integer" severity failure;
    assert describe(2.5) = "real" severity failure;
    assert describe(f) = "flags" severity failure;
    assert describe(green) = "color" severity failure;
    report describe(p) & " " & describe(17) & " " & describe(2.5) & " " & describe(f) & " "
      & describe(green);
    wait;
  end process;
end architecture;
