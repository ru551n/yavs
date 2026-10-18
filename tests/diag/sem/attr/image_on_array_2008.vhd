-- yavs-test: std=2008
-- expect: E1007 5:37 `'image` of an array type mark needs VHDL-2019
-- LRM 16.2
package p is
  constant c : string := bit_vector'image("01");
end package;
