-- yavs-test: std=2019
-- expect: E1621 6:16 attribute `converse` is only defined for mode views
-- LRM 2019 16.2
package p is
  constant c : integer := 1;
  alias x is c'converse;
end package;
