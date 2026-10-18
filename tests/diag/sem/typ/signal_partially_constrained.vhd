-- yavs-test: std=2008
-- expect: E1212 6:10 signal `s` needs a fully constrained subtype
-- LRM 6.4.2.3
package p is
  type mem is array (natural range <>) of bit_vector;
  signal s : mem(0 to 3);
end package;
