-- yavs-test: std=2008
-- expect: E1300 15:16 expected `ip`, found `INTEGER`
-- expect: E1301 16:19 procedure `DEALLOCATE` takes 1 argument, found 2
-- LRM 5.4.3
package p is
  procedure run;
end package;
package body p is
  type ip is access integer;
  procedure run is
    variable p : ip := new integer'(3);
    variable i : integer;
  begin
    deallocate(p);
    deallocate(i);
    deallocate(p, p);
  end procedure;
end package body;
