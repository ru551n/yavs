-- yavs-test: std=2008
-- expect: E1007 6:40 `'value` of a range attribute is not available in VHDL-2008
-- LRM 16.2.6 is new in 2019: R'VALUE needs VHDL-2019 (#282)
package p is
  subtype word_t is integer range 7 downto 0;
  constant y : integer := word_t'range'value.left;
end package;
