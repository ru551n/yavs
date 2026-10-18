-- yavs-test: std=2008
-- expect: none
-- LRM 4.8, 6.4.2.2
package p is
  constant c : integer;
  constant d, e : bit;
end package;
package body p is
  constant c : integer := 1;
  constant d : bit := '0';
  constant e : bit := '1';
end package body;
