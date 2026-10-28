-- yavs-test: std=1993
-- expect: E1007 7:9 an element of an unconstrained array subtype is not allowed in VHDL-1993
-- expect: E1007 9:31 an element of an unconstrained array subtype is not allowed in VHDL-1993
-- LRM 1993 3.2.1, 3.2.1.1: element subtypes of arrays and records are constrained (#814)
package p is
  type r is record
    v : bit_vector;
  end record;
  type a is array (0 to 1) of bit_vector;
  type ok is array (0 to 1) of bit_vector(0 to 3);
end package;
