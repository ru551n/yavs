-- difftest: top=null_deref std=2008 exit=1
entity null_deref is end;
architecture t of null_deref is
  type ip is access integer;
begin
  process
    variable p : ip := null;
  begin
    report "before";
    p.all := 1;
    report "unreachable";
    wait;
  end process;
end;
