-- Clocked processes of every edge form under coverage (crates/yavs/tests/coverage.rs, #707): a
-- process whose only wake-up condition is an edge is resumed at that edge, so the statement of
-- the edge test counts one run at initialisation and one per edge.
library ieee;
use ieee.std_logic_1164.all;

entity cov_edge is
end entity;

architecture sim of cov_edge is
  signal clk, cpy : std_logic := '0';
  signal r, f, l, w, x : natural := 0;
begin
  gen : process
  begin
    for i in 1 to 4 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    wait;
  end process;

  p_rise : process (clk)
  begin
    if rising_edge(clk) then
      r <= r + 1;
    end if;
  end process;

  p_fall : process (clk)
  begin
    if falling_edge(clk) then
      f <= f + 1;
    end if;
  end process;

  p_event : process (clk)
  begin
    if clk'event and clk = '1' then
      l <= l + 1;
    end if;
  end process;

  p_wait : process
  begin
    wait until rising_edge(clk);
    w <= w + 1;
  end process;

  p_level : process
  begin
    wait until clk = '1';
    x <= x + 1;
  end process;

  check : process
  begin
    wait for 100 ns;
    assert r = 4 and f = 4 and l = 4 and w = 4 and x = 4
      report "edge counts " & integer'image(r) & " " & integer'image(f) & " " & integer'image(l)
        & " " & integer'image(w) & " " & integer'image(x)
      severity failure;
    wait;
  end process;

  -- A plain copy: the kernel runs it itself only when no statement counter observes it.
  cpy <= clk;
end architecture;
