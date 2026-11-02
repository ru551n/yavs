-- difftest: top=settle_tree std=2008 stop=60ns
-- Pure combinational processes fed through paths of different depth, read only at a clock
-- edge (#1064): the shape settle-phase deferral is for. Results must not change.
library ieee;
use ieee.std_logic_1164.all;
entity settle_tree is end;
architecture t of settle_tree is
  signal clk : std_ulogic := '0';
  signal r, s1, s2, s3, t1, t2, o : std_ulogic_vector(7 downto 0) := (others => '0');
  signal acc : std_ulogic_vector(7 downto 0) := (others => '0');
begin
  clk <= not clk after 5 ns;
  reg : process (clk) begin
    if rising_edge(clk) then
      r <= r(6 downto 0) & not r(7);
      acc <= acc xor o;
    end if;
  end process;
  p1 : process (r) begin s1 <= r xor x"5a"; end process;
  p2 : process (s1) begin s2 <= s1(3 downto 0) & s1(7 downto 4); end process;
  p3 : process (s2) begin s3 <= not s2; end process;
  q1 : process (r, s3) begin t1 <= r and s3; end process;
  q2 : process (s1, t1) begin t2 <= s1 or t1; end process;
  q3 : process (t2, s3, r) begin o <= t2 xor s3 xor r; end process;
  show : process (clk) begin
    if falling_edge(clk) then report "acc=" & to_hstring(acc); end if;
  end process;
end;
