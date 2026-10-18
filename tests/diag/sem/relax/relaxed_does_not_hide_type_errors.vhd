-- yavs-test: std=2008 flags=--relaxed
-- expect: E1300 5:27 expected `integer`, found `bit`
-- LRM 12.3
package p is
  constant c : integer := '1';
end package;
