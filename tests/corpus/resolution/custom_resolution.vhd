-- difftest: top=custom_resolution std=2008
entity custom_resolution is end;
architecture t of custom_resolution is
  type int_vec is array (natural range <>) of integer;
  function maxres(v : int_vec) return integer is
    variable m : integer := integer'low;
  begin
    for i in v'range loop if v(i) > m then m := v(i); end if; end loop;
    return m;
  end;
  subtype rint is maxres integer;
  signal s : rint := 0;
begin
  s <= 3 after 1 ns, 9 after 5 ns;
  s <= 7 after 2 ns, 1 after 8 ns;
  s <= 5 after 3 ns;
  process (s) begin report "s=" & integer'image(s) & " @" & time'image(now); end process;
end;
