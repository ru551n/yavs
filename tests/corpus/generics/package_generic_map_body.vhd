-- difftest: top=package_generic_map_body ref=hand
-- The body of a package with a generic map aspect sees the mapped generics (4.7, 4.9, #605), in
-- a library, nested in a package and in a process. ref=hand: sim1 crashes at elaboration and
-- sim2 gives the generic its default value in the body.
package pgmb_lib is
  generic (g : integer := 1);
  generic map (g => 2);
  function f return integer;
end package;
package body pgmb_lib is
  function f return integer is begin return g; end function;
end package body;

package pgmb_outer is
  package gm is
    generic (g : integer := 1; type t);
    generic map (g => 3, t => bit);
    function f return integer;
    constant dc : integer;
    type pt is protected
      procedure inc;
      impure function get return integer;
    end protected;
    function v return t;
  end package;
end package;
package body pgmb_outer is
  package body gm is
    constant dc : integer := g * 10;
    function f return integer is begin return g; end function;
    type pt is protected body
      variable n : integer := g;
      procedure inc is begin n := n + 1; end procedure;
      impure function get return integer is begin return n; end function;
    end protected body;
    function v return t is
      variable x : t;
    begin
      return x;
    end function;
  end package body;
end package body;

entity package_generic_map_body is end;
architecture a of package_generic_map_body is
  shared variable sv : work.pgmb_outer.gm.pt;
begin
  process
    package lp is
      generic (k : integer := 5);
      generic map (k => 7);
      function h return integer;
    end package;
    package body lp is
      function h return integer is begin return k + 1; end function;
    end package body;
  begin
    sv.inc;
    report integer'image(work.pgmb_lib.f) & " " & integer'image(work.pgmb_outer.gm.f) & " "
      & integer'image(work.pgmb_outer.gm.dc) & " " & integer'image(sv.get) & " "
      & bit'image(work.pgmb_outer.gm.v) & " " & integer'image(lp.h);
    wait;
  end process;
end architecture;
