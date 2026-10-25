-- difftest: top=wait_edge std=2008 ignore=order
-- (ignore=order: processes that resume in one cycle report in an order the LRM leaves open.)
-- `wait until` conditions with a clock edge as an `and` conjunct: the process may resume only in
-- a cycle with that edge, so events of the other signals of the wait in other cycles must change
-- nothing. Covered: events of the other signal in the edge's cycle and after it, an edge that
-- changes from wait to wait (rising then falling, two clocks), an `or` (no edge is required), a
-- timeout, `wait on ... until`, a procedure with the wait, `x'event and x = '1'`, a postponed
-- process, and a clock that is an element of an array. Every wake-up is reported with its time.
library ieee;
use ieee.std_logic_1164.all;

entity wait_edge is end;

architecture t of wait_edge is
  signal clk, clk2 : std_logic := '0';
  signal clkb : bit := '0';
  signal clks : std_logic_vector(0 to 1) := "00";
  signal en, flag, a, b : std_logic := '0';
  signal n : natural := 0;
  signal done : boolean := false;

  procedure wait_en (signal c : in std_logic; signal e : in std_logic; tag : in string) is
  begin
    wait until rising_edge(c) and e = '1';
    report tag & " proc at " & time'image(now) & " n=" & integer'image(n);
  end procedure;
begin
  clk <= not clk after 5 ns when not done;
  clkb <= not clkb after 5 ns when not done;
  clk2 <= not clk2 after 7 ns when not done;
  clks(1) <= not clks(1) after 5 ns when not done;

  -- `en` changes in the cycle of a rising edge (25, 35, 45, ...) and between edges (12, 27, 57).
  en <= '1' after 10 ns, '0' after 12 ns, '1' after 25 ns, '0' after 27 ns, '1' after 35 ns,
        '0' after 45 ns, '1' after 55 ns, '0' after 57 ns, '1' after 65 ns, '0' after 75 ns,
        '1' after 85 ns;
  flag <= '1' after 33 ns, '0' after 36 ns, '1' after 63 ns, '0' after 66 ns;
  a <= '1' after 14 ns, '0' after 41 ns, '1' after 56 ns;
  b <= '1' after 20 ns, '0' after 45 ns, '1' after 62 ns;

  count : process (clk)
  begin
    if rising_edge(clk) then
      n <= n + 1;
    end if;
  end process;

  stop : process
  begin
    wait for 100 ns;
    done <= true;
    wait;
  end process;

  -- The plain case, in a loop.
  p1 : process
  begin
    for i in 1 to 6 loop
      wait until rising_edge(clk) and en = '1';
      report "p1 " & integer'image(i) & " at " & time'image(now);
    end loop;
    wait;
  end process;

  -- The edge changes from wait to wait.
  p2 : process
  begin
    for i in 1 to 4 loop
      wait until rising_edge(clk) and en = '1';
      report "p2 rise " & time'image(now);
      wait until falling_edge(clk) and en = '1';
      report "p2 fall " & time'image(now);
    end loop;
    wait;
  end process;

  -- No edge is required: the flag alone resumes it.
  p3 : process
  begin
    for i in 1 to 5 loop
      wait until rising_edge(clk) or flag = '1';
      report "p3 " & integer'image(i) & " at " & time'image(now);
    end loop;
    wait;
  end process;

  -- With a timeout.
  p4 : process
  begin
    for i in 1 to 6 loop
      wait until en = '1' and rising_edge(clk) for 7 ns;
      report "p4 " & integer'image(i) & " at " & time'image(now) & " timeout " & boolean'image(en /= '1');
    end loop;
    wait;
  end process;

  -- `wait on`: only an event of `en` in the cycle of an edge resumes it.
  p5 : process
  begin
    for i in 1 to 3 loop
      wait on en until rising_edge(clk);
      report "p5 " & integer'image(i) & " at " & time'image(now);
    end loop;
    wait;
  end process;

  -- A procedure with the wait, two processes, two different clocks.
  p6 : process
  begin
    for i in 1 to 3 loop
      wait_en(clk, en, "p6");
    end loop;
    wait;
  end process;

  p7 : process
  begin
    for i in 1 to 3 loop
      wait_en(clk2, a, "p7");
    end loop;
    wait;
  end process;

  -- The legacy form.
  p8 : process
  begin
    for i in 1 to 5 loop
      wait until clkb'event and clkb = '1' and en = '1';
      report "p8 " & integer'image(i) & " at " & time'image(now);
    end loop;
    wait;
  end process;

  -- Two clocks, one after the other.
  p9 : process
  begin
    for i in 1 to 4 loop
      wait until rising_edge(clk) and a = '1';
      report "p9 clk " & time'image(now);
      wait until rising_edge(clk2) and b = '1';
      report "p9 clk2 " & time'image(now);
    end loop;
    wait;
  end process;

  -- A postponed process.
  p10 : postponed process
  begin
    for i in 1 to 3 loop
      wait until rising_edge(clk) and en = '1';
      report "p10 " & integer'image(i) & " at " & time'image(now);
    end loop;
    wait;
  end process;

  -- The clock is an element of an array.
  p11 : process
  begin
    for i in 1 to 4 loop
      wait until rising_edge(clks(1)) and en = '1';
      report "p11 " & integer'image(i) & " at " & time'image(now);
    end loop;
    wait;
  end process;

  -- The same wait executed again and again by one process with a plain wait between.
  p12 : process
  begin
    for i in 1 to 3 loop
      wait until rising_edge(clk) and a = '1';
      wait for 3 ns;
      report "p12 " & integer'image(i) & " at " & time'image(now);
    end loop;
    wait;
  end process;
end architecture;
