-- yavs-test: std=2008
-- expect: E1007 6:29 `'reflect` is not available in VHDL-2008
-- LRM 2019 16.2
package p is
  constant c : integer := 1;
  constant d : boolean := c'reflect = null;
end package;
