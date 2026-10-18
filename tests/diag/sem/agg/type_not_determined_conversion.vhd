-- yavs-test: std=2008
-- expect: E1320 11:21 the type of this aggregate cannot be determined
-- LRM 9.3.3
package p is
  procedure q;
end package;
package body p is
  procedure q is
    variable v : bit_vector(0 to 1);
  begin
    v := bit_vector(('0', '1'));
  end procedure;
end package body;
