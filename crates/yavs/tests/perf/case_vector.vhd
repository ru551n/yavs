-- A `case` on a logic vector with many choices dispatches on one key (#1051).
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity case_vector is end entity;
architecture a of case_vector is
  signal clk : std_ulogic := '0';
  signal op : std_ulogic_vector(3 downto 0) := "0000";
  signal res : natural := 0;
begin
  clk <= not clk after 5 ns when now < 10 us;
  process (clk)
    variable n : natural := 0;
  begin
    if rising_edge(clk) then
      op <= std_ulogic_vector(unsigned(op) + 1);
      case op is
        when "0000" => n := n + 1;
        when "0001" => n := n + 2;
        when "0010" => n := n + 3;
        when "0011" => n := n + 4;
        when "0100" => n := n + 5;
        when "0101" => n := n + 6;
        when "0110" => n := n + 7;
        when "0111" => n := n + 8;
        when "1000" => n := n + 9;
        when "1001" => n := n + 10;
        when "1010" => n := n + 11;
        when "1011" => n := n + 12;
        when "1100" => n := n + 13;
        when "1101" => n := n + 14;
        when "1110" => n := n + 15;
        when others => n := n + 16;
      end case;
      res <= n;
    end if;
  end process;
  process
  begin
    wait for 10 us + 1 ns;
    report "n=" & integer'image(res);
    wait;
  end process;
end architecture;
