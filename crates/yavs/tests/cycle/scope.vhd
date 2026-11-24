-- Scopes of --two-state[=PATH] and --cycle[=PATH] (A64): one cell entity in the design under
-- test, in a bus model and in a generate, so its architecture is lowered in both variants. A cell
-- in the two-state scope starts its register at 0 and gives 0 for the 'X' it returns; outside it
-- keeps 'U' and the 'X'. The bus model checks its 'Z' line, which it sees in the scope as 0.
library ieee;
use ieee.std_logic_1164.all;

entity sc_cell is
  port (
    clk  : in  std_logic;
    line : in  std_logic;
    q    : out std_logic;
    zq   : out std_logic
  );
end entity;

architecture rtl of sc_cell is
  signal r : std_logic;
  -- A subprogram of the architecture and one of a process: each variant has its own (an 'X'
  -- returned two-state is 0).
  function pick(x : std_logic) return std_logic is
  begin
    if x = '1' then
      return '1';
    end if;
    return 'X';
  end function;
begin
  process (clk)
  begin
    if rising_edge(clk) then
      r <= not r;
    end if;
  end process;
  q  <= r;
  process (line)
    procedure put(v : std_logic) is
    begin
      zq <= v;
    end procedure;
  begin
    put(pick(line));
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity sc_dut is
  port (
    clk  : in  std_logic;
    line : in  std_logic;
    q    : out std_logic;
    zq   : out std_logic
  );
end entity;

architecture rtl of sc_dut is
begin
  c : entity work.sc_cell port map (clk, line, q, zq);
end architecture;

-- A bus model: a behavioural process and a cell of its own.
library ieee;
use ieee.std_logic_1164.all;

entity sc_vc is
  port (
    clk  : in  std_logic;
    line : inout std_logic;
    q    : out std_logic;
    zq   : out std_logic
  );
end entity;

architecture sim of sc_vc is
  signal seen : std_logic;
begin
  line <= 'Z';
  c : entity work.sc_cell port map (clk, line, q, zq);
  process
  begin
    wait for 1 ns;
    seen <= '1' when line = 'Z' else '0';
    wait for 1 ns;
    report "vc sees " & std_logic'image(seen);
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity scope_tb is
end entity;

architecture tb of scope_tb is
  signal clk, line : std_logic := '0';
  signal q, zq     : std_logic_vector(3 downto 0);
begin
  dut : entity work.sc_dut port map (clk, line, q(0), zq(0));
  vc  : entity work.sc_vc port map (clk, line, q(1), zq(1));
  g : for i in 0 to 1 generate
    cell : entity work.sc_cell port map (clk, line, q(2 + i), zq(2 + i));
  end generate;
  process
  begin
    line <= 'Z';
    wait for 3 ns;
    report "q=" & to_string(q) & " zq=" & to_string(zq);
    wait;
  end process;
end architecture;
