-- difftest: top=formal_packages std=2008
-- Formal packages: generic map (<>), generic map (default), a full explicit map
-- and a partial one (the generics left out take their defaults).
package pair_g is
  generic (a : integer := 1; b : integer := 2);
  function combine return integer;
end package;
package body pair_g is
  function combine return integer is
  begin
    return a * 10 + b;
  end function;
end package body;

package typed_g is
  generic (type t; first : t; function image(x : t) return string);
  function show return string;
end package;
package body typed_g is
  function show return string is
  begin
    return "<" & image(first) & ">";
  end function;
end package body;

package any_g is
  generic (package p is new work.pair_g generic map (<>));
  function show return integer;
end package;
package body any_g is
  function show return integer is
  begin
    return p.combine + p.a + p.b;
  end function;
end package body;

package any_typed_g is
  generic (package p is new work.typed_g generic map (<>));
  function show return string;
  function again(x : p.t) return string;
end package;
package body any_typed_g is
  function show return string is
  begin
    return p.show & p.image(p.first);
  end function;
  function again(x : p.t) return string is
  begin
    return p.image(x) & p.image(x);
  end function;
end package body;

package dflt_g is
  generic (package p is new work.pair_g generic map (default));
  function show return integer;
end package;
package body dflt_g is
  function show return integer is
  begin
    return p.combine * 2;
  end function;
end package body;

package full_g is
  generic (package p is new work.pair_g generic map (a => 4, b => 5));
  function show return integer;
end package;
package body full_g is
  function show return integer is
  begin
    return p.combine + 1;
  end function;
end package body;

package part_g is
  generic (package p is new work.pair_g generic map (a => 9));
  function show return integer;
end package;
package body part_g is
  function show return integer is
  begin
    return p.combine + 1000;
  end function;
end package body;

package images is
  function int_image(x : integer) return string;
  function chr_image(x : character) return string;
end package;
package body images is
  function int_image(x : integer) return string is
  begin
    return integer'image(x);
  end function;
  function chr_image(x : character) return string is
  begin
    return character'image(x);
  end function;
end package body;

package pair_78 is new work.pair_g generic map (a => 7, b => 8);
package pair_dflt is new work.pair_g;
package pair_45 is new work.pair_g generic map (a => 4, b => 5);
package pair_9 is new work.pair_g generic map (a => 9);
package typed_int is new work.typed_g generic map (t => integer, first => 42, image => work.images.int_image);
package typed_chr is new work.typed_g generic map (t => character, first => 'x', image => work.images.chr_image);
package any_78 is new work.any_g generic map (p => work.pair_78);
package any_dflt is new work.any_g generic map (p => work.pair_dflt);
package dflt_x is new work.dflt_g generic map (p => work.pair_dflt);
package full_x is new work.full_g generic map (p => work.pair_45);
package part_x is new work.part_g generic map (p => work.pair_9);

entity formal_packages is end entity;
architecture t of formal_packages is
  package local_pair is new work.pair_g generic map (a => 3);
  package local_any is new work.any_g generic map (p => local_pair);
  package local_part is new work.part_g generic map (p => work.pair_9);
begin
  process
  begin
    report "any_78=" & integer'image(work.any_78.show);
    report "any_dflt=" & integer'image(work.any_dflt.show);
    report "dflt_x=" & integer'image(work.dflt_x.show);
    report "full_x=" & integer'image(work.full_x.show);
    report "part_x=" & integer'image(work.part_x.show);
    report "local_any=" & integer'image(local_any.show) & " local_part=" & integer'image(local_part.show);
    wait;
  end process;
end architecture;
