-- yavs-test: std=2008
-- expect: E1622 7:24 `s` is a signal, not a variable
-- LRM 7.2
package p is
  attribute tag : string;
  signal s : bit;
  attribute tag of s : variable is "x";
end package;
