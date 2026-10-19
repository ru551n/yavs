-- yavs-test: std=2008
-- expect: E1237 12:13 an instance in a process declares a signal
-- LRM 4.9, 11.3
entity e is end;
architecture a of e is
  package g is
    generic (n : integer);
    signal s : integer := n;
  end package;
begin
  process
    package i is new g generic map (n => 1);
  begin
    wait;
  end process;
end;
