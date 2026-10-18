-- yavs-test: std=2008
-- expect: E1621 7:29 `s` has no value for attribute `tag`
-- LRM 7.2
package p is
  signal s : bit;
  attribute tag : integer;
  constant c : integer := s'tag;
end package;
