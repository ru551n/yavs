-- yavs-test: std=2019
-- expect: none
-- LRM 2019 6.5.3: anonymous index and element types, an index constraint
package g is
  generic (type vt is array (type is range <>) of type is private);
  subtype et is vt'element;
  subtype it is vt'index;
  function last_of (v : vt) return et;
end package;
package body g is
  function last_of (v : vt) return et is
    variable j : it := v'left;
  begin
    for i in v'range loop j := i; end loop;
    return v(j + 0);
  end function;
end package body;
package iv is new work.g generic map (vt => integer_vector);
package bv is new work.g generic map (vt => bit_vector);
package q is
  generic (type ct is array (0 to 3) of type is (<>);
           type pt is access type is private;
           type ft is file of type is private);
  function second (c : ct) return ct'element;
  subtype ps is pt;
  procedure rd (file f : ft);
end package;
package body q is
  function second (c : ct) return ct'element is begin return c(1); end function;
  procedure rd (file f : ft) is begin end procedure;
end package body;
package u is
  type quad is array (0 to 3) of character;
  type line_ptr is access string;
  type text_file is file of string;
end package;
package qi is new work.q generic map (ct => work.u.quad, pt => work.u.line_ptr, ft => work.u.text_file);
entity mux is
  generic (type et is array (type is (<>)) of type is private);
  port (sel : in et'index; inputs : in et; output : out et'element);
end entity;
architecture a of mux is
begin
  output <= inputs(sel);
end architecture;
