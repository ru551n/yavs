-- yavs-test: std=2019
-- expect: none
-- LRM 2019 6.5.3
package g is
  generic (type e; type a is array (natural range <>) of e);
subtype s is a; end package;
package i is new work.g generic map (e => bit, a => bit_vector);
