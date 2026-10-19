-- An instance of a generic package that declares a signal, in a process: the instance is
-- a package declaration there (LRM 4.9), and a process declares no signals (4.7, 11.3).
package g is
  generic (k : integer);
  signal s : integer := k;
end package;
entity top is end;
architecture a of top is
begin
  process
    package i is new work.g generic map (k => 1);
  begin
    report "RES " & integer'image(i.s);
    wait;
  end process;
end;
