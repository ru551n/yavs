-- Cycle mode: a variable assigned an integer just computed and then given to the element of an
-- array of one integer keeps its value. The element's word was taken for a fresh value, so the
-- operation that wrote the variable was made to write the element instead and the variable kept
-- its old value. Found auditing the first fuzzer fix.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_intelem1 is
end entity;
architecture a of cycle_intelem1 is
  type ia is array (0 to 0) of integer range 0 to 1000;
  signal clk : std_ulogic := '0';
  signal i : integer range 0 to 100 := 3;
  signal q, q2, q3 : integer range 0 to 2000 := 0;
  signal arr_s : ia := (others => 0);
begin
  clk <= not clk after 5 ns;

  regs : process (clk)
    variable v : integer range 0 to 1000 := 0;
    variable arr : ia := (others => 0);
  begin
    if rising_edge(clk) then
      v := i + 1;
      arr(0) := v;
      q <= v + arr(0);
      v := i + 2;
      arr_s(0) <= v;
      q2 <= v;
    end if;
  end process;

  comb : process (all)
    variable w : integer range 0 to 1000;
    variable arr : ia;
  begin
    w := i * 3;
    arr(0) := w;
    q3 <= w + arr(0) + 1;
  end process;

  stim : process
  begin
    for k in 0 to 5 loop
      wait until falling_edge(clk);
      report "q=" & integer'image(q) & " q2=" & integer'image(q2) & " q3=" & integer'image(q3)
        & " s=" & integer'image(arr_s(0));
      i <= (i * 7 + 5) mod 101;
    end loop;
    std.env.finish;
  end process;
end architecture;
