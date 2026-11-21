-- difftest: top=copy_back_range_uplevel std=2008 exit=1 ref=hand
-- ref=hand: sim1 copies the value back without a check (discrepancy `copy-back-range`); sim2
-- agrees with this output. A procedure of the process reads the actual by its own name while
-- the out formal holds 10: the actual still has its value from before the call, which is checked
-- after it (LRM 4.2.2.2, #1175).
entity copy_back_range_uplevel is end entity;
architecture a of copy_back_range_uplevel is
begin
  process
    variable q : integer range 0 to 9 := 5;
    procedure set (variable o : out integer; v : integer) is
    begin
      o := v;
      report "q=" & integer'image(q);
    end procedure;
  begin
    set(q, 7);
    report "q=" & integer'image(q);
    set(q, 10);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
