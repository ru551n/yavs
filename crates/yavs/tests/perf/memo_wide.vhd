-- 16 clocked processes, each xoring 48 instance-constant taps of a vector (q((i+7+3j) mod N)), 40 cycles:
-- one memoised block with more than 40 values and over 800 instructions (opt/memo.rs caps).
library ieee; use ieee.std_logic_1164.all;
entity memo_wide is end;
architecture a of memo_wide is
  constant N : natural := 16;
  signal clk : std_logic := '0';
  signal q : std_logic_vector(N-1 downto 0) := "0110100111010010";
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done else clk;
  g : for i in 0 to N-1 generate
    process (clk) begin
      if rising_edge(clk) then
        q(i) <= q((i+1) mod N) xor (q(i) nor q((i+3) mod N)) xor q((i+7) mod N) xor q((i+10) mod N) xor q((i+13) mod N) xor q((i+16) mod N) xor q((i+19) mod N) xor q((i+22) mod N) xor q((i+25) mod N) xor q((i+28) mod N) xor q((i+31) mod N) xor q((i+34) mod N) xor q((i+37) mod N) xor q((i+40) mod N) xor q((i+43) mod N) xor q((i+46) mod N) xor q((i+49) mod N) xor q((i+52) mod N) xor q((i+55) mod N) xor q((i+58) mod N) xor q((i+61) mod N) xor q((i+64) mod N) xor q((i+67) mod N) xor q((i+70) mod N) xor q((i+73) mod N) xor q((i+76) mod N) xor q((i+79) mod N) xor q((i+82) mod N) xor q((i+85) mod N) xor q((i+88) mod N) xor q((i+91) mod N) xor q((i+94) mod N) xor q((i+97) mod N) xor q((i+100) mod N) xor q((i+103) mod N) xor q((i+106) mod N) xor q((i+109) mod N) xor q((i+112) mod N) xor q((i+115) mod N) xor q((i+118) mod N) xor q((i+121) mod N) xor q((i+124) mod N) xor q((i+127) mod N) xor q((i+130) mod N) xor q((i+133) mod N) xor q((i+136) mod N) xor q((i+139) mod N) xor q((i+142) mod N) xor q((i+145) mod N) xor q((i+148) mod N);
      end if;
    end process;
  end generate;
  process
    variable s : natural := 0;
  begin
    for c in 1 to 40 loop wait until rising_edge(clk); end loop;
    wait for 1 ns;
    for i in 0 to N-1 loop
      if q(i) = '1' then s := s + 2 ** i; end if;
    end loop;
    done <= true;
    report "q=" & integer'image(s);
    wait;
  end process;
end;
