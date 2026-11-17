-- Edge waits written other ways: a user "and" of a std_ulogic and a BOOLEAN that is the boolean
-- `and` of `l = '1'` and `r` (on signals and on record fields), `std_logic_1164` gates left of
-- the edge, and a concurrent assertion `assert not (rising_edge(clk) and ...)`. Each resumes only
-- on the rising edge, not for the falling edge or the operand events between edges.
library ieee; use ieee.std_logic_1164.all;
package wef_ops is
  type hs_t is record
    ready, valid : std_ulogic;
  end record;
  function "and" (l : std_ulogic; r : boolean) return boolean;
end package;
package body wef_ops is
  function "and" (l : std_ulogic; r : boolean) return boolean is
  begin
    return (l = '1') and r;
  end function;
end package body;

library ieee; use ieee.std_logic_1164.all;
use work.wef_ops.all;
entity wait_edge_forms is end;
architecture a of wait_edge_forms is
  signal clk : std_logic := '0';
  signal ready, valid : std_logic := '0';
  signal hs : hs_t := ('0', '0');
  signal cnt : natural := 0;
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  process (clk)
  begin
    if rising_edge(clk) then
      cnt <= cnt + 1;
      if cnt = 300 then done <= true; end if;
    end if;
  end process;
  -- Operand events between the edges.
  ready <= not ready after 3 ns when not done;
  valid <= not valid after 7 ns when not done;
  hs <= (ready, valid);
  process
    variable hits : natural := 0;
  begin
    for i in 1 to 40 loop
      wait until ready and valid and rising_edge(clk);
      hits := hits + 1;
    end loop;
    report "user=" & integer'image(hits);
    wait;
  end process;
  process
    variable hits : natural := 0;
  begin
    for i in 1 to 40 loop
      wait until (ready and valid) = '1' and rising_edge(clk);
      hits := hits + 1;
    end loop;
    report "gates=" & integer'image(hits);
    wait;
  end process;
  process
    variable hits : natural := 0;
  begin
    for i in 1 to 40 loop
      wait until hs.ready and hs.valid and rising_edge(clk);
      hits := hits + 1;
    end loop;
    report "fields=" & integer'image(hits);
    wait;
  end process;
  assert not (rising_edge(clk) and cnt = 1000 and valid = '1') report "never" severity failure;
end;
