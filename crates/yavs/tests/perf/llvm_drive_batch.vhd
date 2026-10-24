-- Element-wise drives of one vector in a loop under compiled code: the 16 drives of an
-- activation go to the kernel in one call, not one callback each.
library ieee; use ieee.std_logic_1164.all;
entity llvm_drive_batch is end;
architecture a of llvm_drive_batch is
  signal clk : std_logic := '0';
  signal q : std_logic_vector(15 downto 0) := x"0001";
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  process (clk) begin
    if rising_edge(clk) then
      for i in q'range loop
        q(i) <= q((i + 15) mod 16) xor (q(i) and q((i + 3) mod 16));
      end loop;
    end if;
  end process;
  process begin
    wait for 500 ns;
    report "q=" & to_hstring(q);
    done <= true; wait;
  end process;
end;
