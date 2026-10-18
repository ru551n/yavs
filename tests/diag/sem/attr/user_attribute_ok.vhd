-- yavs-test: std=2008
-- expect: none
-- LRM 7.2
package p is
  attribute tag : integer;
  signal s : bit;
  attribute tag of s : signal is 1;
  constant c : integer := s'tag;
end package;
