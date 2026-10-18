-- yavs-test: std=2008
-- expect: E1330 6:22 `integer` has no character literals
-- LRM 15.7
package p is
  type iv is array (0 to 1) of integer;
  constant c : iv := "ab";
end package;
