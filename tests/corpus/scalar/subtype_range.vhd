-- difftest: top=subtype_range std=2008 exit=1
entity subtype_range is end;
architecture t of subtype_range is
  subtype pct is integer range 0 to 100;
begin
  process
    variable p : pct := 100;
  begin
    report "before";
    p := p + 1;
    report "after " & integer'image(p);
    wait;
  end process;
end;
