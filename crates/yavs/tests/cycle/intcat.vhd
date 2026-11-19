-- A pipeline of integers shifted by concatenating an element and a slice of an array of integers
-- (`lanes_q <= lane & lanes_q(1 to N - 1)`), both orders, against the normal mode (`W` replaced
-- by a width: the stages).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_intcat is
end entity;
architecture t of cycle_intcat is
  subtype lane_t is natural range 0 to 255;
  type lanes_t is array (1 to W + 1) of lane_t;
  signal clk : std_ulogic := '0';
  signal lane : lane_t := 0;
  signal fwd, back : lanes_t := (others => 0);
begin
  pipe : process (clk)
  begin
    if rising_edge(clk) then
      fwd <= lane & fwd(1 to W);
      back <= back(2 to W + 1) & lane;
    end if;
  end process;
  main : process
  begin
    for k in 1 to 12 loop
      lane <= (k * 37) mod 256;
      wait for 1 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
      report "k=" & integer'image(k) & " fwd=" & integer'image(fwd(1)) & "," & integer'image(fwd(W + 1))
        & " back=" & integer'image(back(1)) & "," & integer'image(back(W + 1));
    end loop;
    wait;
  end process;
end architecture;
