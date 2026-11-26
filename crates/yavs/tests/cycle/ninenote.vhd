-- A clocked process below the top-level unit reports a register of every pattern of nine values
-- in hexadecimal, octal and element by element: the region's notes print what the normal mode
-- prints (a group of `'Z'` as Z, any other metavalue as X, `L` and `H` as metavalues).
library ieee;
use ieee.std_logic_1164.all;
entity ninenote_dut is
  port (clk : in std_ulogic; d : in std_ulogic_vector(7 downto 0));
end entity;
architecture rtl of ninenote_dut is
  signal q : std_ulogic_vector(7 downto 0);
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= d;
      report "q=" & to_string(q) & " h=" & to_hstring(q) & " o=" & to_ostring(q) & " b=" & std_ulogic'image(q(7));
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
entity cycle_ninenote is
end entity;
architecture a of cycle_ninenote is
  signal clk : std_ulogic := '0';
  signal d : std_ulogic_vector(7 downto 0);
  type pats is array (natural range <>) of std_ulogic_vector(7 downto 0);
  constant p : pats := ("ZZZZZZZZ", "1Z0ZZZZZ", "LLLLHHHH", "UUUU0000", "XZ01WLH-", "ZZZ01ZZZ", "--------",
                        "00110101");
begin
  dut : entity work.ninenote_dut port map (clk, d);
  main : process
  begin
    for i in p'range loop
      d <= p(i);
      wait for 1 ns;
      clk <= '1';
      wait for 1 ns;
      clk <= '0';
    end loop;
    wait;
  end process;
end architecture;
