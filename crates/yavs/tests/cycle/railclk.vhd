-- Clocked choices on a 'U' without --two-state: each image takes its own branch. The region's
-- image reads 'U' as 0, so `s = '0'`, `unsigned(ad) < 2` and `when "00"` are true there while the
-- normal mode takes the other branch (a known value). Following the region's branch on the second
-- image too would copy `uu` (0 there, 1 on the second) and show 'U' (Microwatt's icache, A43).
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity cycle_railclk is
  generic (W : positive := 8);
end;
architecture a of cycle_railclk is
  signal clk : std_ulogic := '0';
  signal s : std_ulogic;
  signal ad : std_ulogic_vector(1 downto 0);
  signal uu : std_ulogic_vector(W - 1 downto 0);
  signal zero : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal q0, qn, qc : std_ulogic_vector(W - 1 downto 0);
begin
  clk <= not clk after 5 ns when now < 100 ns else clk;
  ck : process (clk) begin
    if rising_edge(clk) then
      if s = '0' then
        q0 <= uu;
      else
        q0 <= zero;
      end if;
      if unsigned(ad) < 2 then
        qn <= uu;
      else
        qn <= zero;
      end if;
      case ad is
        when "00" => qc <= uu;
        when others => qc <= zero;
      end case;
    end if;
  end process;
  stim : process begin
    wait for 22 ns;
    report "q0=" & to_string(q0) & " qn=" & to_string(qn) & " qc=" & to_string(qc);
    wait;
  end process;
end;
