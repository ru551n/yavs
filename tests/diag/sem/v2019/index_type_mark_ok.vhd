-- yavs-test: std=2019
-- expect: none
-- LRM 16.2 (2019): A'INDEX and A'INDEX(n) denote an index subtype (#174)
package p is
  type big_t is array (natural range <>, positive range <>, character range <>) of integer;
  signal idx1 : big_t'index(1);
  signal idx3 : big_t'index(3);
  constant c1 : big_t'index := 0;
  signal s : bit_vector(3 downto 0);
  constant c2 : s'index := 2;
end package;
