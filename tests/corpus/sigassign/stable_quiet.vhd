-- difftest: top=stable_quiet std=2008
entity stable_quiet is end;
architecture t of stable_quiet is
  signal s : integer := 0;
  signal st : boolean;
  signal q : boolean;
begin
  st <= s'stable(2 ns);
  q <= s'quiet(2 ns);
  process begin
    wait for 5 ns; s <= 1;
    wait for 10 ns; s <= 2;
    wait;
  end process;
  process (st, q) begin
    if now > 0 ns then report "stable=" & boolean'image(st) & " quiet=" & boolean'image(q) & " @" & time'image(now); end if;
  end process;
end;
