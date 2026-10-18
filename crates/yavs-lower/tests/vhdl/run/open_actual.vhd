-- difftest: top=open_actual std=2008
entity open_actual is end;
architecture t of open_actual is
  function f(a : integer := 5; b : integer := 7) return integer is
  begin
    return a * 10 + b;
  end;
begin
  process
  begin
    report integer'image(f(a => open, b => 1)) & " " & integer'image(f(2, open));
    wait;
  end process;
end;
