-- `p'delayed(1 ns)` of an `in` port whose declared initial value (INTEGER'LEFT) differs from
-- its actual's (0): its value before the first event.
entity child is
  port (p : in integer);
end;
architecture a of child is
begin
  process begin
    report "RES delayed " & integer'image(p'delayed(1 ns)) & " at 0 ns";
    wait for 2 ns;
    report "RES delayed " & integer'image(p'delayed(1 ns)) & " at 2 ns";
    wait;
  end process;
end;

entity top is end;
architecture a of top is
  signal s : integer := 0;
begin
  u : entity work.child port map (p => s);
end;
