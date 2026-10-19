-- LRM 14.7.2, 8.7: a procedure declared in a process drives an external signal with the
-- process's driver: one driver, as if the process assigned it.
entity top is end;
architecture t of top is
  signal mirror : integer := -1;
begin
  main : process
    procedure copy(v : integer) is
    begin
      <<signal .top.mirror : integer>> <= v;
    end procedure;
  begin
    copy(3);
    wait for 1 ns;
    report "RES " & integer'image(mirror);
    wait;
  end process;
end;
