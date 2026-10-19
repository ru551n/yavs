-- yavs-test: std=2008
-- expect: E1237 9:14 a package in a process declares a signal
-- LRM 4.7, 11.3
entity e is end;
architecture a of e is
begin
  process
    package p is
      signal s : integer;
    end package;
  begin
    wait;
  end process;
end;
