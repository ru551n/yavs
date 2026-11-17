-- difftest: top=const_elem_ranges std=2008
-- Elements of constant arrays of vectors take the element subtype's index range (LRM 5.3.2.2,
-- 6.4.2.2), not the range of the literal that gave them (a bit-string literal is `0 to n - 1`):
-- bits and static slices of them read during elaboration (signal initial values, constants,
-- port actuals) and at run time, both directions, ranges not from 0, positional and `others`
-- aggregates, from a function, in a package and an architecture. Elaboration had read them by
-- the literal's range (a reversed bit, "slice direction differs from its prefix").
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
package const_elem_ranges_pkg is
  type rom_d is array (17 downto 2) of signed(18 downto 3);
  constant RD : rom_d := ("1111000010001010", "0010010100111110", x"D7FD", "0000010100011010",
                          "1011110110011101", "1011011110100101", "1101111110101101", x"888B",
                          "1001011110101010", "1101001010001000", "0110001000000101", x"25A8",
                          "1010101100101110", "0000100000010101", x"1234", x"8001");
  type rom_a is array (1 to 4) of unsigned(8 to 15);
  constant RA : rom_a := (others => x"5A");
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity const_elem_ranges_sub is
  port (a : in std_ulogic_vector(3 downto 0); b : in std_ulogic);
end entity;
architecture a of const_elem_ranges_sub is
begin
  process
  begin
    report "port a=" & to_hstring(a) & " b=" & std_ulogic'image(b);
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.const_elem_ranges_pkg.all;
entity const_elem_ranges is end entity;
architecture a of const_elem_ranges is
  function f return std_ulogic_vector is
  begin
    return x"01";
  end function;
  type rom_l is array (0 to 3) of std_ulogic_vector(7 downto 0);
  constant RF : rom_l := (others => f);
  constant RL : rom_l := (x"01", x"80", "00001111", x"F0");
  constant C1 : std_ulogic := '0' nor RD(17)(3);
  constant C2 : unsigned(3 downto 0) := unsigned(RD(17)(6 downto 3));
  signal s1 : std_ulogic := '0' nor RD(17)(3);
  signal s2 : std_ulogic := RD(2)(18);
  signal s3 : signed(3 downto 0) := RD(9)(10 downto 7);
  signal s4 : unsigned(0 to 3) := RA(2)(9 to 12);
  signal s5 : std_ulogic := RL(0)(0);
  signal s6 : std_ulogic_vector(2 downto 0) := RL(1)(7 downto 5);
  signal s7 : std_ulogic := RF(1)(0);
  signal s8 : std_ulogic_vector(1 downto 0) := RF(0)(1 downto 0);
begin
  u : entity work.const_elem_ranges_sub port map (a => RL(2)(3 downto 0), b => RL(3)(7));
  process
    variable v : std_ulogic_vector(3 downto 0);
  begin
    v := std_ulogic_vector(RD(16)(6 downto 3));
    report "C1=" & std_ulogic'image(C1) & " C2=" & to_hstring(C2) & " s1=" & std_ulogic'image(s1)
      & " s2=" & std_ulogic'image(s2) & " s3=" & to_hstring(s3) & " s4=" & to_hstring(s4)
      & " s5=" & std_ulogic'image(s5) & " s6=" & to_hstring(s6) & " s7=" & std_ulogic'image(s7)
      & " s8=" & to_string(s8) & " v=" & to_hstring(v)
      & " rt=" & std_ulogic'image('0' nor RD(17)(3)) & std_ulogic'image(RL(0)(0));
    wait;
  end process;
end architecture;
