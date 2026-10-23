-- An 8-bit unsigned counter and a 16-bit xor register: logic vectors committed directly (plane
-- words), numeric_std arithmetic native.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity reg_planes is end;
architecture a of reg_planes is
  signal clk : std_logic := '0';
  signal c : unsigned(7 downto 0) := (others => '0');
  signal x : std_logic_vector(15 downto 0) := x"0001";
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  process (clk) begin
    if rising_edge(clk) then c <= c + 1; end if;
  end process;
  process (clk) begin
    if rising_edge(clk) then x <= x(14 downto 0) & (x(15) xor x(13)); end if;
  end process;
  process begin
    wait for 1 us;
    report "c=" & integer'image(to_integer(c)) & " x=" & to_hstring(x);
    done <= true; wait;
  end process;
end;
