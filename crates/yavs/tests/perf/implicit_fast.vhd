-- Implicit signals (`'delayed`, `'stable`, `'quiet`, `'transaction`) of a few signals in a design
-- of clocked processes and generators: the updates of the other signals take the fast paths, and
-- the implicit signals still see every event and transaction of their sources, those of signals
-- committed by the fast paths too (a register that a clocked process drives, a clock).
library ieee;
use ieee.std_logic_1164.all;

entity implicit_fast is end;
architecture a of implicit_fast is
  signal clk : std_logic := '0';
  signal reg : integer := 0;
  signal slow : std_logic := '0';
  type arr is array (0 to 7) of integer;
  signal d_clk, d_slow : integer := 0;
  signal st_reg, qu_slow, tr_reg : integer := 0;
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  slow <= not slow after 37 ns when not done;
  g : for i in 0 to 7 generate
    signal r : integer := 0;
  begin
    process (clk) begin
      if rising_edge(clk) then r <= r + i; end if;
    end process;
  end generate;
  process (clk) begin
    if rising_edge(clk) then
      reg <= reg + 1;
      if reg = 150 then done <= true; end if;
    end if;
  end process;
  p1 : process begin
    wait on clk'delayed(2 ns);
    d_clk <= d_clk + 1;
  end process;
  p2 : process begin
    wait on slow'delayed(11 ns);
    d_slow <= d_slow + 1;
  end process;
  p3 : process (reg, reg'stable(7 ns)) begin
    if reg'stable(7 ns) then st_reg <= st_reg + 1; end if;
  end process;
  p4 : process (slow'quiet(5 ns)) begin
    if slow'quiet(5 ns) then qu_slow <= qu_slow + 1; end if;
  end process;
  p5 : process (reg'transaction) begin
    tr_reg <= tr_reg + 1;
  end process;
  mon : process (done)
  begin
    if done then
      report "d_clk=" & integer'image(d_clk) & " d_slow=" & integer'image(d_slow)
        & " st=" & integer'image(st_reg) & " qu=" & integer'image(qu_slow) & " tr=" & integer'image(tr_reg);
    end if;
  end process;
end;
