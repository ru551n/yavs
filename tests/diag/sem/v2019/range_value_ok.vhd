-- yavs-test: std=2019
-- expect: none
-- LRM 16.2.6 (2019): R'VALUE is the range record of the range attribute name R (#282)
package p is
  type mat_t is array (natural range <>, integer range <>) of bit;
  subtype word_t is integer range 7 downto 0;
  constant a : natural'range'record := word_t'range'value;
  constant b : integer'range'record := word_t'reverse_range'value;
  function f (m : mat_t; v : bit_vector) return natural'range'record;
end package;
package body p is
  function f (m : mat_t; v : bit_vector) return natural'range'record is
    constant c : integer'range'record := m'range(2)'value;
    constant d : natural'range'record := v'reverse_range'value;
  begin
    return v'range'value when v'ascending else d;
  end function;
end package body;
