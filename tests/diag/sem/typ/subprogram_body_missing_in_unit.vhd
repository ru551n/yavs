-- yavs-test: std=2008
-- expect: E1230 5:13 procedure `pr` has no body
-- LRM 4.3
package p is
  procedure pr;
end package;
package body p is
  constant k : integer := 1;
end package body;
