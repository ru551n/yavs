-- (from the differential corpus)
-- ref=hand: the reference simulator crashes when lowering a call of a formal
-- subprogram through a formal package (`p.image`); the third reference
-- simulator agrees with the expected output.
-- Types, constants and subprograms of a formal package used inside the generic.
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
    variable v : p.t := x;
  begin
    return p.image(v) & p.image(x);
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

package typed_int is new work.typed_g generic map (t => integer, first => 42, image => work.images.int_image);
package typed_chr is new work.typed_g generic map (t => character, first => 'x', image => work.images.chr_image);
package any_int is new work.any_typed_g generic map (p => work.typed_int);
package any_chr is new work.any_typed_g generic map (p => work.typed_chr);

entity formal_package_subprograms is end entity;
architecture t of formal_package_subprograms is
begin
  process
  begin
    report "any_int=" & work.any_int.show & " " & work.any_int.again(-5);
    report "any_chr=" & work.any_chr.show & " " & work.any_chr.again('q');
    wait;
  end process;
end architecture;
