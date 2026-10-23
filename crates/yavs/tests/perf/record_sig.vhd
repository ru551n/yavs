-- A record signal with vector fields read and written by a clocked process, and a wide (256-bit)
-- vector of which one bit changes per cycle: events and activity are kept per block.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity record_sig is end;
architecture a of record_sig is
  type rec is record
    addr : unsigned(31 downto 0);
    data : std_logic_vector(31 downto 0);
    valid : std_logic;
  end record;
  signal clk : std_logic := '0';
  signal r : rec := (addr => (others => '0'), data => x"DEADBEEF", valid => '0');
  signal wide : std_logic_vector(255 downto 0) := (others => '0');
  signal seen : natural := 0;
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  process (clk) begin
    if rising_edge(clk) then
      r.addr <= r.addr + 4;
      r.data <= r.data(30 downto 0) & r.data(31);
      r.valid <= not r.valid;
      wide(to_integer(r.addr(9 downto 2)) mod 256) <= not wide(to_integer(r.addr(9 downto 2)) mod 256);
    end if;
  end process;
  process (wide) begin seen <= seen + 1; end process;
  process begin
    wait for 500 ns;
    report "addr=" & to_hstring(std_logic_vector(r.addr)) & " data=" & to_hstring(r.data) & " seen=" & integer'image(seen);
    done <= true; wait;
  end process;
end;
