-- yavs-test: std=2008
-- expect: E1621 6:29 attribute `event` requires a signal prefix
-- LRM 16.2
package p is
  constant k : bit := '0';
  constant c : boolean := k'event;
end package;
