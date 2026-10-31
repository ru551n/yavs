-- difftest: top=alias_subtype_attr std=2019
-- An alias of an attribute that denotes a subtype, on a type, an object and a generic array
-- type (6.6, 16.2, #914).
package asa_p is
  type arr_t is array (natural range <>) of bit;
  type acc_t is access arr_t;
  alias idx_t is arr_t'index;
  alias el_t  is arr_t'element;
  alias des_t is acc_t'designated_subtype;
  constant c : arr_t(0 to 1) := "01";
  alias c_st is c'subtype;
end package;
package asa_p2 is
  generic (type array_type is array (type is (<>)) of type is private);
  alias index_type   is array_type'index;
  alias element_type is array_type'element;
  function first_elem (a : array_type) return element_type;
end package;
package body asa_p2 is
  function first_elem (a : array_type) return element_type is
    variable i : index_type := a'left;
  begin return a(i); end function;
end package body;
use work.asa_p.all;
entity alias_subtype_attr is end;
architecture a of alias_subtype_attr is
  package q is new work.asa_p2 generic map (array_type => arr_t);
  signal e : el_t := '1';
  signal x : idx_t := 3;
  signal s : c_st;
begin
  process begin
    report bit'image(e) & " " & integer'image(x) & " " & integer'image(s'length) & " " & bit'image(q.first_elem("10"));
    wait;
  end process;
end architecture;
