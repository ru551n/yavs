library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity counter is
  port (
    clk, rst : in  std_logic;
    en       : in  std_logic;
    count    : out unsigned(7 downto 0);
    wrapped  : out std_logic
  );
end entity;

architecture rtl of counter is
  signal value : unsigned(7 downto 0) := (others => '0');
begin
  -- Clocked, with an asynchronous reset.
  p_count : process (clk, rst)
  begin
    if rst = '1' then
      value <= (others => '0');
    elsif rising_edge(clk) then
      if en = '1' then
        value <= value + 1;
      end if;
    end if;
  end process;

  -- Concurrent assignments are combinational logic.
  count   <= value;
  wrapped <= '1' when value = 255 else '0';
end architecture;
