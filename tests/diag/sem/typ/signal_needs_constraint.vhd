-- yavs-test: std=2008
-- expect: E1212 6:10 signal `s` needs a fully constrained subtype
-- help: give the signal a constrained subtype, e.g. `bit_vector(7 downto 0)`
-- LRM 6.4.2.3
package p is
  signal s : bit_vector;
end package;
