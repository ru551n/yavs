-- difftest: top=generic_arrays_2019 std=2019
package vec_pkg is
  generic (type vt is array (type is range <>) of type is private);
  subtype et is vt'element;
  subtype it is vt'index;
  function first_of(v : vt) return et;
  function count(v : vt) return natural;
  function rev(v : vt) return vt;
end package;
package body vec_pkg is
  function first_of(v : vt) return et is begin return v(v'left); end;
  function count(v : vt) return natural is
    variable n : natural := 0;
  begin
    for i in v'range loop n := n + 1; end loop;
    return n;
  end;
  function rev(v : vt) return vt is
    variable r : vt(v'range);
    variable j : it := v'right;
  begin
    for i in v'range loop r(i) := v(j); if i /= v'right then j := j - 1; end if; end loop;
    return r;
  end;
end package body;

package int_vec is new work.vec_pkg generic map (vt => integer_vector);
package bit_vec is new work.vec_pkg generic map (vt => bit_vector);

entity mux is
  generic (type et is array (type is (<>)) of type is private);
  port (sel : in et'index; inputs : in et; output : out et'element);
end entity;
architecture a of mux is
begin
  output <= inputs(sel);
end architecture;

entity generic_arrays_2019 is end;
architecture t of generic_arrays_2019 is
  type color is (red, green, blue);
  type by_color is array (color) of integer;
  function width
    generic (type at is array (natural range <>) of type is private)
    parameter (l : at) return natural is
  begin
    return l'length;
  end function;
  function w_iv is new width generic map (at => integer_vector);
  function w_s is new width generic map (at => string);
  signal sel : color := red;
  signal o : integer;
  constant ins : by_color := (10, 20, 30);
begin
  u : entity work.mux generic map (et => by_color) port map (sel, ins, o);
  process
  begin
    report integer'image(work.int_vec.first_of((7, 8, 9)));
    report integer'image(work.int_vec.count((7, 8, 9)));
    report integer'image(work.int_vec.rev((7, 8, 9))(0));
    report bit'image(work.bit_vec.first_of("0110"));
    report integer'image(w_iv((1, 2, 3, 4))) & " " & integer'image(w_s("hello"));
    wait for 1 ns;
    report integer'image(o);
    sel <= blue;
    wait for 1 ns;
    report integer'image(o);
    wait;
  end process;
end architecture;
