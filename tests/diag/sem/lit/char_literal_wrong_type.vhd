-- yavs-test: std=2008
-- expect: E1330 5:23 `'a'` is not a literal of type `bit`
-- LRM 15.6
package p is
  constant c : bit := 'a';
end package;
