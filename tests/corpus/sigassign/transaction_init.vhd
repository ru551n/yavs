-- difftest: top=transaction_init std=2008 ref=hand
-- ref=hand: two reference simulators toggle 'transaction in delta 1 of time 0, and one of them
-- starts 'delayed(T) at the type's default value until T has passed. LRM 14.7.5.2: at
-- initialisation a signal's value is assumed to have been its value for an infinite time, so
-- no signal is active in the first cycles and 'transaction keeps its initial '0' until the
-- first transaction; 'stable(T) and 'quiet(T) start TRUE and 'delayed(T) at the prefix's
-- initial value. The third reference simulator gives these values (it prints no delta or path,
-- and reports of one cycle from different processes in another order) (#241).
entity transaction_init is
end entity;

architecture test of transaction_init is
  signal n  : natural := 1;
  signal tr : bit;
  signal dl : natural;
begin
  tr <= n'transaction;
  dl <= n'delayed(2 ns);

  drv : process
  begin
    wait for 1 ns;
    n <= 5;
    wait for 2 ns;
    n <= 5;
    wait;
  end process;

  mon : process (tr) is
  begin
    report "tr " & bit'image(tr);
  end process;

  mond : process (dl) is
  begin
    report "dl " & integer'image(dl);
  end process;

  direct : process
  begin
    for k in 1 to 2 loop
      report "transaction " & bit'image(n'transaction)
        & " stable " & boolean'image(n'stable(1 ns))
        & " quiet " & boolean'image(n'quiet)
        & " delayed " & integer'image(n'delayed(2 ns));
      wait for 0 ns;
    end loop;
    wait on n'transaction;
    report "first transaction " & bit'image(n'transaction)
      & " delayed " & integer'image(n'delayed(2 ns));
    wait on n'transaction;
    report "second transaction " & bit'image(n'transaction)
      & " delayed " & integer'image(n'delayed(2 ns));
    wait;
  end process;
end architecture;
