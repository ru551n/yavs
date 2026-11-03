-- A wide bus nothing asks the history of: no attribute, no signal parameter, so the kernel keeps no
-- event record and no 'last_value for it, here read whole, in part through a wait and copied.
library ieee; use ieee.std_logic_1164.all;
entity bare_bus is end;
architecture a of bare_bus is
  signal clk : std_logic := '0';
  signal b, c : std_logic_vector(199 downto 0) := (0 => '1', 7 => '1', others => '0');
  signal n : natural := 0;
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  c <= b;
  process (clk) begin
    if rising_edge(clk) then
      n <= n + 1;
      b <= b(198 downto 0) & (b(199) xor b(5));
    end if;
  end process;
  process
    variable ones, parts : natural := 0;
  begin
    while not done loop
      wait on c(120 downto 60), done;
      parts := parts + 1;
      for k in c'range loop
        if c(k) = '1' then ones := ones + 1; end if;
      end loop;
    end loop;
    report "parts=" & integer'image(parts) & " ones=" & integer'image(ones);
    wait;
  end process;
  done <= true when n = 100;
end;
