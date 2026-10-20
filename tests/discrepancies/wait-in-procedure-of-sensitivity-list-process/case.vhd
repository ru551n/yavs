-- A procedure containing a wait, called by a process with a sensitivity list (LRM 11.3).
entity top is end;
architecture a of top is
  signal s : bit;
  procedure pause is
  begin
    wait for 1 ns;
  end procedure;
begin
  process (s)
  begin
    pause;
    report "after pause";
  end process;
end;
