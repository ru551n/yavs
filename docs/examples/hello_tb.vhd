library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use std.env.all;

entity hello_tb is
  generic (
    GREETING : string  := "hello";
    EXPECTED : natural := 3
  );
end entity;

architecture sim of hello_tb is
  signal clk   : std_logic := '0';
  signal count : unsigned(3 downto 0) := (others => '0');
begin
  clk <= not clk after 5 ns;

  counter : process (clk)
  begin
    if rising_edge(clk) then
      count <= count + 1;
    end if;
  end process;

  check : process
  begin
    wait for 32 ns;
    report GREETING & ", count is " & integer'image(to_integer(count));
    assert to_integer(count) = EXPECTED
      report "expected " & integer'image(EXPECTED) & ", got " & integer'image(to_integer(count))
      severity failure;
    report "done";
    finish;
  end process;
end architecture;
