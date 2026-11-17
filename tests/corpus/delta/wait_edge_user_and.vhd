-- difftest: top=wait_edge_user_and std=2008 ignore=order
-- (ignore=order: processes that resume in one cycle report in an order the LRM leaves open.)
-- `wait until` conditions that join a clock edge with a user-defined "and" of a `std_ulogic` and a
-- BOOLEAN. When the body is the boolean `and` of `l = '1'` and `r`, the process may resume only in
-- a cycle with the edge; any other body is evaluated at every event. Covered: both operand orders,
-- metavalue and weak left operands, record fields, a falling edge, the predefined `and` with `std_logic_1164`
-- gates left of the edge, and bodies that are not that `and` (an `or`, `not l`, a `bit` operand
-- with a report, a report before the `and`), whose wake-ups without an edge must stay. Every
-- wake-up is reported.
library ieee;
use ieee.std_logic_1164.all;

package user_ops is
  type hs_t is record
    ready, valid : std_ulogic;
  end record;
  function "and" (l : std_ulogic; r : boolean) return boolean;
  function "and" (l : boolean; r : std_ulogic) return boolean;
end package;

package body user_ops is
  function "and" (l : std_ulogic; r : boolean) return boolean is
  begin
    return (l = '1') and r;
  end function;

  function "and" (l : boolean; r : std_ulogic) return boolean is
  begin
    return l and ('1' = r);
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;

package other_ops is
  -- Look like the ones above but are not the boolean `and` of `l = '1'` and `r`.
  function "and" (l : std_ulogic; r : boolean) return boolean;
  function "and" (l : boolean; r : std_ulogic) return boolean;
  function "and" (l : bit; r : boolean) return boolean;
end package;

package body other_ops is
  function "and" (l : std_ulogic; r : boolean) return boolean is
  begin
    return (l = '1') or r;
  end function;

  function "and" (l : boolean; r : std_ulogic) return boolean is
  begin
    return (r = '1') and not l;
  end function;

  function "and" (l : bit; r : boolean) return boolean is
  begin
    report "bit and evaluated";
    return (l = '1') and r;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;

package rep_ops is
  function "and" (l : std_ulogic; r : boolean) return boolean;
end package;

package body rep_ops is
  -- The boolean `and` with a report in front.
  function "and" (l : std_ulogic; r : boolean) return boolean is
  begin
    report "reporting and evaluated";
    return (l = '1') and r;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;

entity wait_edge_user_and is end;

architecture t of wait_edge_user_and is
  signal clk : std_logic := '0';
  signal ready, valid : std_logic := '0';
  signal weak : std_logic := 'U';
  signal bv : bit := '0';
  signal hs : work.user_ops.hs_t := ('0', '0');
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  ready <= '1' after 12 ns, '0' after 27 ns, '1' after 33 ns, '0' after 52 ns, '1' after 58 ns;
  valid <= '1' after 14 ns, '0' after 21 ns, '1' after 25 ns, '0' after 47 ns, '1' after 55 ns;
  weak <= 'X' after 10 ns, 'H' after 20 ns, '1' after 30 ns, 'Z' after 40 ns, '-' after 50 ns,
          '1' after 60 ns, 'U' after 70 ns;
  bv <= '1' after 13 ns, '0' after 26 ns, '1' after 44 ns;
  done <= true after 100 ns;
  hs.ready <= '1' after 13 ns, '0' after 32 ns, '1' after 44 ns, 'X' after 62 ns, '1' after 71 ns;
  hs.valid <= '1' after 16 ns, 'H' after 35 ns, '1' after 46 ns;

  -- The boolean `and` with the edge as its right operand (`ready and valid` is std_ulogic).
  p1 : process
    use work.user_ops."and";
    variable k : natural := 0;
  begin
    while not done loop
      wait until ready and valid and rising_edge(clk);
      k := k + 1;
      report "p1 " & integer'image(k) & " at " & time'image(now);
    end loop;
    wait;
  end process;

  -- The edge as the left operand, a falling edge.
  p2 : process
    use work.user_ops."and";
    variable k : natural := 0;
  begin
    while not done loop
      wait until falling_edge(clk) and ready;
      k := k + 1;
      report "p2 " & integer'image(k) & " at " & time'image(now);
    end loop;
    wait;
  end process;

  -- Metavalues and weak values on the std_ulogic operand: `l = '1'` is false for them.
  p3 : process
    use work.user_ops."and";
    variable k : natural := 0;
  begin
    while not done loop
      wait until weak and rising_edge(clk);
      k := k + 1;
      report "p3 " & integer'image(k) & " at " & time'image(now) & " weak " & std_ulogic'image(weak);
    end loop;
    wait;
  end process;

  -- The edge inside a predefined `and` chain that is the boolean operand.
  p4 : process
    use work.user_ops."and";
    variable k : natural := 0;
  begin
    while not done loop
      wait until ready and (valid = '1' and rising_edge(clk));
      k := k + 1;
      report "p4 " & integer'image(k) & " at " & time'image(now);
    end loop;
    wait;
  end process;

  -- Record fields as operands, as in a bus handshake.
  p6 : process
    use work.user_ops."and";
    variable k : natural := 0;
  begin
    while not done loop
      wait until hs.ready and hs.valid and rising_edge(clk);
      k := k + 1;
      report "p6 " & integer'image(k) & " at " & time'image(now);
    end loop;
    wait;
  end process;

  -- Predefined `and` with std_logic_1164 gates left of the edge (no user operator).
  p5 : process
    variable k : natural := 0;
  begin
    while not done loop
      wait until (ready and valid) = '1' and not weak = '0' and rising_edge(clk);
      k := k + 1;
      report "p5 " & integer'image(k) & " at " & time'image(now);
    end loop;
    wait;
  end process;

  -- An `or` body: true without the edge whenever ready is '1'.
  n1 : process
    use work.other_ops."and";
    variable k : natural := 0;
  begin
    while not done and k < 12 loop
      wait until ready and rising_edge(clk);
      k := k + 1;
      report "n1 " & integer'image(k) & " at " & time'image(now) & " clk " & std_ulogic'image(clk);
    end loop;
    wait;
  end process;

  -- `(r = '1') and not l`: true without the edge whenever ready is '1'.
  n2 : process
    use work.other_ops."and";
    variable k : natural := 0;
  begin
    while not done and k < 12 loop
      wait until rising_edge(clk) and ready;
      k := k + 1;
      report "n2 " & integer'image(k) & " at " & time'image(now) & " ready " & std_ulogic'image(ready);
    end loop;
    wait;
  end process;

  -- A body that reports: each evaluation shows.
  n3 : process
    use work.other_ops."and";
    variable k : natural := 0;
  begin
    while not done and k < 4 loop
      wait until bv and rising_edge(clk);
      k := k + 1;
      report "n3 " & integer'image(k) & " at " & time'image(now);
    end loop;
    wait;
  end process;

  -- A body with a report before the `and`: each evaluation shows.
  n4 : process
    use work.rep_ops."and";
    variable k : natural := 0;
  begin
    while not done and k < 4 loop
      wait until valid and rising_edge(clk);
      k := k + 1;
      report "n4 " & integer'image(k) & " at " & time'image(now);
    end loop;
    wait;
  end process;
end architecture;
