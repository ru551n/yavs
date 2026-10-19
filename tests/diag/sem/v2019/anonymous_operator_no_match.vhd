-- yavs-test: std=2019
-- expect: E1301 11:8 no visible `"+"` accepts these operands
-- LRM 2019 6.5.3: an operand of a parameter of anonymous integer type is of an integer type
package p is
  function "+" (a : type is range <>; b : a'subtype) return a'subtype;
end package;
entity top is end;
architecture a of top is
  signal s : bit_vector(1 downto 0);
begin
  s <= "00" + "01";
end architecture;
