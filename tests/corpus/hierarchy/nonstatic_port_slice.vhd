-- difftest: top=tb_nonstatic_port_slice std=2008
-- A slice whose bounds read a signal as the actual of an `in` port and of a part, both
-- directions: an expression actual, driven one delta after the signal changes, at the
-- bounds of that moment. The slice had been taken once, at the bounds of elaboration (found by
-- the cycle mode fuzzer), and the port then did not follow the signal. A `wait until` on such a
-- slice waits on the signal of its bounds too.
library ieee;
use ieee.std_logic_1164.all;
entity sub_nsps is
  port (b : in std_ulogic_vector(1 downto 0); c : in std_ulogic_vector(3 downto 0);
        u : in std_ulogic_vector(2 downto 0));
end entity;
architecture a of sub_nsps is
begin
  process (b, c, u)
  begin
    report "b=" & to_string(b) & " c=" & to_string(c) & " u=" & to_string(u);
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
entity tb_nonstatic_port_slice is end entity;
architecture a of tb_nonstatic_port_slice is
  signal x : std_ulogic_vector(9 downto 0) := "0110101010";
  signal y : std_ulogic_vector(0 to 9) := "0011100110";
  signal k : integer range 0 to 6 := 0;
begin
  u : entity work.sub_nsps
    port map (b => x(k + 1 downto k), c(1 downto 0) => x(k + 2 downto k + 1), c(3 downto 2) => "10",
              u => y(k to k + 2));
  process
  begin
    for i in 1 to 5 loop
      wait for 1 ns;
      k <= i;
    end loop;
    wait;
  end process;
  waiter : process
  begin
    wait until x(k + 1 downto k) = "01";
    report "x(k + 1 downto k) = 01 at k=" & integer'image(k);
  end process;
end architecture;
