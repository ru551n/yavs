-- Logic vectors of 200 and 40 bits read from signals by compiled code in place (any length), a
-- reduction over the copy, and driven back: one read per vector, not per element.
library ieee; use ieee.std_logic_1164.all;
entity llvm_sigread is end;
architecture a of llvm_sigread is
  signal clk : std_logic := '0';
  signal wide : std_logic_vector(199 downto 0) := (199 => '1', 0 => '1', others => '0');
  signal mid : std_logic_vector(39 downto 0) := x"00000000F1";
  signal parity : std_logic := '0';
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  process (clk)
    variable v : std_logic_vector(199 downto 0);
    variable p : std_logic;
  begin
    if rising_edge(clk) then
      wide <= wide(198 downto 0) & wide(199);
      mid <= mid(38 downto 0) & mid(39);
      v := wide;
      p := xor v;
      parity <= p xor mid(0);
    end if;
  end process;
  process begin
    wait for 500 ns;
    report "wide=" & to_hstring(wide) & " mid=" & to_hstring(mid) & " parity=" & std_logic'image(parity);
    done <= true; wait;
  end process;
end;
