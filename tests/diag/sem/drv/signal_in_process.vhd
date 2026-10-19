-- yavs-test: std=2008
-- expect: E1237 8:12 a signal declared in a process
-- LRM 11.3
entity e is end;
architecture a of e is
begin
  process
    signal s : integer;
  begin
    wait;
  end process;
end;
