-- yavs-test: std=2019
-- expect: E1201 5:14 prefix is a type, not a range attribute
-- LRM 16.2.6 (2019): the prefix of 'RECORD is a range attribute name
package p is
  signal r : integer'record;
end package;
