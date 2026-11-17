-- Cycle mode: globally static calls whose arguments are bits and slices of elements of constant
-- arrays (folded from elaboration's values, `region_calls`): an element has its subtype's index
-- range, also when an aggregate of bit-string literals gave it (`0 to n - 1`). Read by the
-- literal's range, `'0' nor RD(17)(3)` came out '0' in a clocked member. Found by the fuzzer.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
package cycle_constcall_pkg is
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
use work.cycle_constcall_pkg.all;
entity cycle_constcall is
end entity;
architecture a of cycle_constcall is
  type rom_l is array (0 to 3) of std_ulogic_vector(7 downto 0);
  constant RL : rom_l := (x"01", x"80", "00001111", x"F0");
  signal clk, x : std_ulogic := '0';
  signal q1, q2, c1, c2 : std_ulogic := '0';
  signal q3, c3 : unsigned(3 downto 0) := x"0";
  signal q4 : unsigned(0 to 3) := x"0";
begin
  clk <= not clk after 5 ns;

  regs : process (clk)
  begin
    if rising_edge(clk) then
      q1 <= ('0' nor RD(17)(3)) xor x;
      q2 <= (RL(0)(0) and RD(2)(18)) xor x;
      q3 <= (unsigned(RD(17)(6 downto 3)) + 1) xor (3 downto 0 => x);
      q4 <= (RA(2)(9 to 12) xor "1111") xor (0 to 3 => x);
    end if;
  end process;

  c1 <= ('0' nor RD(17)(3)) and not x;
  c2 <= RL(1)(7) xor RD(9)(10);
  c3 <= unsigned(RD(9)(10 downto 7)) - 1;

  stim : process
  begin
    for i in 0 to 5 loop
      wait until falling_edge(clk);
      report "q1=" & std_ulogic'image(q1) & " q2=" & std_ulogic'image(q2) & " q3=" & to_hstring(q3)
        & " q4=" & to_hstring(q4) & " c1=" & std_ulogic'image(c1) & " c2=" & std_ulogic'image(c2)
        & " c3=" & to_hstring(c3);
      x <= not x;
    end loop;
    std.env.finish;
  end process;
end architecture;
