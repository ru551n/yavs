-- difftest: top=no_wait_infinite exit=1 ref=hand spin=3000000 ignore=stderr
-- ref=hand: the error text is yavs's; the references spin forever or are killed.
-- ignore=stderr: only yavs warns (W3102, its text names the file path). spin=3000000: the bound
-- is 10^9 jumps back as shipped; a lower one keeps the run to seconds (#583).
-- A process without a wait statement or sensitivity list never suspends, so its activation is
-- bounded and reported instead of spinning forever at time 0 (#583).
entity no_wait_infinite is end entity;
architecture a of no_wait_infinite is
  signal a : bit := '0';
begin
  p : process
  begin
    a <= not a;
  end process;
end architecture;
