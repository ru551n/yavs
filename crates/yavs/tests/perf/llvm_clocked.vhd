-- Clocked registers under compiled code: each activation reads its signals in place, drives and
-- waits again; the callbacks per activation are what the native entry work minimises.
library ieee; use ieee.std_logic_1164.all;
entity llvm_clocked is end;
architecture a of llvm_clocked is
  signal clk : std_logic := '0';
  signal q : std_logic_vector(15 downto 0) := x"0001";
  signal b : std_logic_vector(15 downto 0) := (others => '0');
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  g: for i in 0 to 15 generate
    process (clk) begin
      if rising_edge(clk) then
        q(i) <= q((i + 15) mod 16) xor (q(i) and q((i + 3) mod 16));
      end if;
    end process;
  end generate;
  process (clk) begin
    if rising_edge(clk) then b <= q xor b; end if;
  end process;
  process begin
    wait for 500 ns;
    report "q=" & to_hstring(q) & " b=" & to_hstring(b);
    done <= true; wait;
  end process;
end;
