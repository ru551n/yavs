-- Slices and signal references whose bounds depend only on generics and generate parameters:
-- their positions are computed once per instance (memoised), not on every activation.
library ieee; use ieee.std_logic_1164.all;
entity slice_memo is generic (G : natural := 3); end;
architecture a of slice_memo is
  signal clk : std_logic := '0';
  signal v : std_logic_vector(0 to 15) := x"A5C3";
  signal o : std_logic_vector(0 to 15) := (others => '0');
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  gen: for i in 0 to 3 generate
    process (clk) begin
      if rising_edge(clk) then
        o(i*4 to i*4+3) <= v((i+G) mod 4 * 4 to (i+G) mod 4 * 4 + 3) xor v(i*4 to i*4+3);
      end if;
    end process;
  end generate;
  process begin
    wait for 500 ns;
    report "o=" & to_hstring(o);
    done <= true; wait;
  end process;
end;
