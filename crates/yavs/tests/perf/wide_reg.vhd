-- A 200-bit rotating register and a 130-bit one that is rewritten with the same value: wide
-- logic vectors committed directly, one entry per plane word (no general assignment).
library ieee; use ieee.std_logic_1164.all;
entity wide_reg is end;
architecture a of wide_reg is
  signal clk : std_logic := '0';
  signal q : std_logic_vector(199 downto 0) := (0 => '1', 7 => '1', others => '0');
  signal k : std_logic_vector(129 downto 0) := (others => '1');
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  process (clk) begin
    if rising_edge(clk) then q <= q(198 downto 0) & (q(199) xor q(5)); end if;
  end process;
  process (clk) begin
    if rising_edge(clk) then k <= (others => '1'); end if;
  end process;
  process begin
    wait for 1 us;
    report "q=" & to_hstring(q) & " k=" & to_hstring(k);
    done <= true; wait;
  end process;
end;
