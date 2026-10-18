-- difftest: top=cond_2019 std=2019
entity cond_2019 is end;
architecture t of cond_2019 is
begin
  process
    variable n : integer := 2;
    variable s : string(1 to 3);
  begin
    s := "one" when n = 1 else "two" when n = 2 else "etc";
    n := 5 when n > 1 else 6;
    report s & " " & integer'image(n);
    wait;
  end process;
end;
