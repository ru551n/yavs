-- difftest: top=loops_exit_next std=2008
entity loops_exit_next is end;
architecture t of loops_exit_next is
begin
  process
    variable n : integer := 0;
  begin
    outer : for i in 1 to 5 loop
      for j in 1 to 5 loop
        next outer when j > i;
        exit outer when i = 4;
        n := n + 1;
      end loop;
    end loop;
    report "n=" & integer'image(n);
    while n > 0 loop n := n - 4; end loop;
    report "n=" & integer'image(n);
    l : loop n := n + 1; exit l when n >= 3; end loop;
    for i in 10 downto 7 loop report "down " & integer'image(i); end loop;
    for i in 5 to 1 loop report "never"; end loop;
    report "n=" & integer'image(n);
    wait;
  end process;
end;
