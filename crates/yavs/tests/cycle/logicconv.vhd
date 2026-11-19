-- Conversions of a logic scalar to its own type (`std_ulogic(x)`, `std_logic(x)`), as bus
-- bridges write them between port types, in concurrent assignments and a clocked process with an
-- asynchronous reset (`W` replaced by a width), and of a vector with no name to its subtype; a
-- conversion to a range of the type (`X01`) checks its value and keeps its process in the kernel.
-- Against the normal mode.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_logicconv is
end entity;
architecture a of cycle_logicconv is
  signal clk, rstn : std_ulogic := '0';
  signal ready : std_logic := '0';
  signal busy : std_ulogic := '0';
  signal valid, last, checked : std_ulogic := '0';
  signal axi_valid : std_logic;
  signal u : unsigned(W - 1 downto 0) := (others => '0');
  signal v, w : std_logic_vector(W - 1 downto 0);
begin
  axi_valid <= std_logic(valid and busy);
  v <= std_logic_vector(u) xor (W - 1 downto 0 => std_ulogic(ready));
  checked <= X01(ready);
  -- A vector with no name of a logic type converted to its subtype.
  w <= std_logic_vector(std_ulogic_vector(u) and std_ulogic_vector(v));

  reg : process (rstn, clk)
  begin
    if rstn = '0' then
      valid <= '0';
      last <= '0';
    elsif rising_edge(clk) then
      if busy = '0' then
        valid <= std_ulogic(ready);
      else
        valid <= valid and std_ulogic(not ready);
      end if;
      last <= std_ulogic(axi_valid) xor last;
    end if;
  end process;

  main : process
  begin
    for k in 1 to 24 loop
      rstn <= '1';
      ready <= '1' when k mod 3 = 1 else '0';
      busy <= '1' when k mod 5 < 2 else '0';
      u <= u + 7 * k;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
      report "k=" & integer'image(k) & " v=" & to_hstring(v) & " w=" & to_hstring(w) & " " & std_ulogic'image(axi_valid)
        & std_ulogic'image(last) & std_ulogic'image(checked);
    end loop;
    wait;
  end process;
end architecture;
