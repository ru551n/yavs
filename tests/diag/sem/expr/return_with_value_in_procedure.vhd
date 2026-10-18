-- yavs-test: std=2008
-- expect: E1802 10:5 a procedure cannot return a value
-- LRM 10.13
package p is
  procedure pr;
end package;
package body p is
  procedure pr is
  begin
    return 1;
  end procedure;
end package body;
