-- yavs-test: std=2008
-- expect: E1361 11:10 `.all` of an integer
-- LRM 8.3
package p is
  procedure run;
end package;
package body p is
  procedure run is
    variable n : integer;
  begin
    n := n.all;
  end procedure;
end package body;
