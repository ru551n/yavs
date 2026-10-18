-- difftest: top=sig_out std=2008
entity sig_out is end;
architecture t of sig_out is
  signal a, b : integer := 0;
  signal f : bit := '0';
  procedure set(signal s : out integer; v : integer) is
  begin
    s <= v after 1 ns;
  end;
  procedure bump(signal s : inout integer) is
  begin
    set(s, s + 10);
  end;
  procedure toggle(signal x : inout bit) is
  begin
    x <= not x;
  end;
begin
  process
  begin
    set(a, 5);
    bump(b);
    toggle(f);
    wait for 2 ns;
    report "a=" & integer'image(a) & " b=" & integer'image(b) & " f=" & bit'image(f);
    bump(a);
    wait for 2 ns;
    report "a=" & integer'image(a);
    wait;
  end process;
end;
