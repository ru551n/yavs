-- A variable of a process passed by reference to a nested procedure that may wait, so it runs in
-- frame mode: the procedure reaches the enclosing frame inline, not through a callback per access.
entity clift_nested_byref is end;
architecture a of clift_nested_byref is
begin
  process
    variable c : integer := 0;
    procedure bump(variable x : inout integer) is
    begin
      for i in 1 to 100 loop
        x := x + 1;
      end loop;
      wait for 1 ns;
    end procedure;
  begin
    for i in 1 to 4 loop
      bump(c);
    end loop;
    report "c=" & integer'image(c);
    wait;
  end process;
end;
