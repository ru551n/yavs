-- yavs-test: std=2008
-- expect: W1403 8:45 static value is outside subtype `bitrange`
-- expect: W1403 9:27 static value is outside subtype `NATURAL`
-- LRM 9.3.5, 9.3.6: the code may never run, so the check is made again when it does
package p is
  subtype bitrange is natural range 0 to 1;
  constant ok : integer := bitrange'(1);
  function f return integer is begin return bitrange'(5); end function;
  constant c : integer := natural(-1.5);
end package;
