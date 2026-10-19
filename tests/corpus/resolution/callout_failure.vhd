-- difftest: top=callout_failure std=2008 exit=1
-- A failure report in a resolution function ends the run like any other failure report,
-- after the cycle it was made in (#372).
entity callout_failure is end;
architecture a of callout_failure is
  type int_vec is array (natural range <>) of integer;
  function sumres(v : int_vec) return integer is
    variable m : integer := 0;
  begin
    for i in v'range loop m := m + v(i); end loop;
    if m = 5 then
      report "five" severity failure;
    end if;
    return m;
  end;
  subtype rint is sumres integer;
  signal r : rint;
begin
  process
  begin
    r <= 3 after 1 ns;
    wait for 2 ns;
    r <= 5;
    wait for 1 ns;
    report "not reached after the cycle";
    wait;
  end process;
end;
