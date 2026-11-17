-- Reports of constants' initial values under --cycle: each made once, as in the normal mode.
-- The value passes of cycle mode evaluate the constants too, apart from the model.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
package cycle_constrep_p is
  function f(x : integer) return integer;
  function g(x : integer) return integer;
end package;
package body cycle_constrep_p is
  function f(x : integer) return integer is
  begin
    report "f called with " & integer'image(x) severity warning;
    return x + 1;
  end function;
  function g(x : integer) return integer is
  begin
    report "g called with " & integer'image(x) severity error;
    return x + 2;
  end function;
end package body;
use work.cycle_constrep_p.all;
package cycle_constrep_q is
  constant K : integer := f(7);
  constant E : integer;
end package;
package body cycle_constrep_q is
  constant E : integer := f(8);
end package body;
-- An instance whose processes all run cycle-based: its constant's report, or its failure
-- with `bad`, is made as in the normal mode.
library ieee; use ieee.std_logic_1164.all;
entity cycle_constrep_leaf is
  generic (bad : boolean);
  port (clk : in std_ulogic; o : out integer);
end;
architecture a of cycle_constrep_leaf is
  function guard(b : boolean) return integer is
  begin
    assert not b report "leaf constant fails" severity failure;
    report "leaf constant made";
    return 4;
  end function;
  constant K2 : integer := guard(bad);
begin
  r : process (clk) begin
    if rising_edge(clk) then o <= K2; end if;
  end process;
end;
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
use work.cycle_constrep_p.all; use work.cycle_constrep_q.all;
entity cycle_constrep is
  generic (bad : boolean := false);
end;
architecture a of cycle_constrep is
  constant C : integer := f(3);
  type rom_t is array (0 to 3) of integer;
  function mk return rom_t is
    variable r : rom_t;
  begin
    for i in r'range loop r(i) := f(i); end loop;
    return r;
  end function;
  constant ROM : rom_t := mk;
  constant G1 : integer := g(1);
  signal clk : std_ulogic := '0';
  signal q, r : unsigned(7 downto 0) := (others => '0');
  signal idx : integer range 0 to 3 := 0;
  signal lo : integer := 0;
begin
  leaf : entity work.cycle_constrep_leaf generic map (bad => bad) port map (clk => clk, o => lo);
  clk <= not clk after 5 ns when now < 50 ns else clk;
  -- A member.
  m : process (clk) begin
    if rising_edge(clk) then
      idx <= (idx + 1) mod 4;
      q <= to_unsigned(ROM(idx) + C + K + E, 8);
    end if;
  end process;
  -- Calls the function of a constant with an error report: stays in the event kernel.
  o : process (clk) begin
    if rising_edge(clk) then
      r <= to_unsigned(g(1) + G1, 8) + r;
    end if;
  end process;
  chk : process begin
    wait for 60 ns;
    report "q=" & to_string(q) & " r=" & to_string(r) & " lo=" & integer'image(lo);
    wait;
  end process;
end;
