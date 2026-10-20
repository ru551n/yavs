-- Subprogram calls: 3000000 calls of one-line functions (four per iteration, two of them
-- nested), so the cost of entering and leaving a frame dominates.
entity bench_calls is
  generic (N : natural := 750000);
end entity;

architecture sim of bench_calls is
  function inc(a : integer) return integer is
  begin
    return a + 1;
  end function;

  function twice(a : integer) return integer is
  begin
    return inc(inc(a));
  end function;
begin
  process
    variable acc : integer := 0;
  begin
    for k in 1 to N loop
      acc := inc(acc);
      acc := twice(acc) - 1;
    end loop;
    assert acc = 2 * N report "calls: wrong result" severity failure;
    report "calls acc=" & integer'image(acc);
    wait;
  end process;
end architecture;
