-- A procedure local to a process, called in a loop: the process's frame (static link 1) and the
-- inout scalar's region are held at the variant's entry, inline (no callback), and every access
-- of them is a test against the held length.
entity clift_uplevel is end;
architecture a of clift_uplevel is
begin
  process
    variable v, s, u : integer := 0;
    procedure step (x : inout integer) is
    begin
      x := x + 1;
      s := s + u;
    end procedure;
  begin
    for i in 1 to 200 loop
      u := i mod 7;
      step(v);
    end loop;
    report "v=" & integer'image(v) & " s=" & integer'image(s);
    wait;
  end process;
end;
