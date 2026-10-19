-- yavs-test: std=2008
-- expect: E1007 5:30 `'record` of a range attribute is not available in VHDL-2008
-- LRM 16.2.6 (2019): R'RECORD is VHDL-2019 only; 2008 reports it instead of an internal error
package p is
  constant r : natural'range'record := (left => 0, right => 3, direction => ascending);
end package;
