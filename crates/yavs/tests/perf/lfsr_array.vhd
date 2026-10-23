-- 8 16-bit LFSRs in an array signal, one process each, 50 cycles: slices and concatenation of
-- std_logic_vector, element-wise signal assignment.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity lfsr_array is end;
architecture a of lfsr_array is
  signal clk : std_logic := '0';
  type arr is array (natural range <>) of std_logic_vector(15 downto 0);
  signal r : arr(0 to 7);
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  g: for i in 0 to 7 generate
    process (clk) begin
      if rising_edge(clk) then
        if r(i)(0) = 'U' then
          r(i) <= std_logic_vector(to_unsigned(16#ACE1# + i * 37, 16));
        else
          r(i) <= r(i)(14 downto 0) & (r(i)(15) xor r(i)(13) xor r(i)(12) xor r(i)(10));
        end if;
      end if;
    end process;
  end generate;
  process
    variable acc : std_logic_vector(15 downto 0) := (others => '0');
  begin
    wait for 500 ns;
    for i in r'range loop acc := acc xor r(i); end loop;
    report "acc=" & to_hstring(acc);
    done <= true; wait;
  end process;
end;
