-- yavs-test: std=2008
-- expect: E1300 13:14 an allocator of `bit` for an access to `integer`
-- expect: E1300 14:14 a qualified allocator of `bit` for an access to `integer`
-- LRM 9.3.7
package p is
  procedure run;
end package;
package body p is
  type ip is access integer;
  procedure run is
    variable p : ip;
  begin
    p := new bit;
    p := new bit'('1');
  end procedure;
end package body;
