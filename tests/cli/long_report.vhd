-- A process that reports for a long time without waiting (#483), and a design that keeps
-- reporting while time advances (#484): the first reports must reach the output at once.
entity long_report is
  generic (waiting : boolean := false);
end entity;

architecture a of long_report is
begin
  process
    variable x : integer := 0;
    variable n : integer := 0;
  begin
    for j in 1 to 1000000 loop
      n := n + 1;
      report "line " & integer'image(n);
      for i in 1 to 1000000 loop
        x := (x + 1) mod 1000;
      end loop;
      if waiting then
        wait for 1 ns;
      end if;
    end loop;
    wait;
  end process;
end architecture;
