-- Many report and assert statements with formatted messages, as a verification log does.
entity bench_report_heavy is
  generic (N : natural := 100000);
end entity;

architecture sim of bench_report_heavy is
begin
  process
    variable sum : natural := 0;
  begin
    for i in 1 to N loop
      report "scoreboard: transaction " & integer'image(i) & " matched, data=" & to_hstring(bit_vector'(x"BEEF"))
        severity note;
      assert i mod 7 /= 0 report "checkpoint " & integer'image(i) & " at " & time'image(now) severity warning;
      sum := sum + 1;
      if i mod 100 = 0 then
        wait for 10 ns;
      end if;
    end loop;
    report "report_heavy sum=" & integer'image(sum);
    wait;
  end process;
end architecture;
