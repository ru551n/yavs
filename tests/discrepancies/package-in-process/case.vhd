-- A package declared in a process (LRM 11.3: process declarative items include package
-- declarations and bodies).
entity top is end;
architecture a of top is
begin
  process
    package p is
      constant c : integer := 4;
    end package;
  begin
    report "RES " & integer'image(p.c);
    wait;
  end process;
end;
