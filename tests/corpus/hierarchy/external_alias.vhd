-- difftest: top=external_alias std=2008 stop=60ns
-- Aliases of external signal names (LRM 6.6.2, 8.7): a monitor block and a stimulus process
-- read and drive signals of an instance below them through aliases, bound at elaboration.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity dut is port (clk : in std_logic); end;
architecture a of dut is
  signal count : natural := 0;
  signal req : bit := '0';
  signal data : std_logic_vector(7 downto 0) := x"00";
begin
  process (clk) begin
    if rising_edge(clk) then
      count <= count + 1;
      if req = '1' then
        data <= std_logic_vector(unsigned(data) + 3);
      end if;
    end if;
  end process;
  process (req) begin
    report "dut: req=" & bit'image(req);
  end process;
end;
library ieee;
use ieee.std_logic_1164.all;
entity external_alias is end;
architecture tb of external_alias is
  signal clk : std_logic := '0';
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  u : entity work.dut port map (clk => clk);
  mon : block
    alias cnt is <<signal .external_alias.u.count : natural>>;
  begin
    process (cnt) begin
      report "count=" & integer'image(cnt);
    end process;
  end block;
  stim : process
    alias req is <<signal .external_alias.u.req : bit>>;
    alias d is <<signal .external_alias.u.data : std_logic_vector>>;
    alias d0 is <<signal .external_alias.u.data : std_logic_vector(7 downto 0)>>;
  begin
    wait for 12 ns;
    req <= '1';
    wait on d;
    report "data=" & to_hstring(d) & " len=" & integer'image(d'length)
      & " bit0=" & std_logic'image(d0(0));
    wait until d0(3 downto 0) = x"6";
    req <= '0';
    report "data=" & to_hstring(d0);
    wait for 10 ns;
    done <= true;
    wait;
  end process;
end;
