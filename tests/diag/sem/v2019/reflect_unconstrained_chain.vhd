-- yavs-test: std=2019
-- expect: E1621 9:36 attribute `'reflect` does not apply to a type mark of type `int_arr`
-- LRM 2019 16.2: the chain after the attribute must not hide the prefix error (issue 154)
package p is
end package;
package body p is
  type int_arr is array (natural range <>) of integer;
  procedure t is
    constant n : string := int_arr'reflect.to_array.element_subtype.simple_name;
  begin
  end procedure;
end package body;
