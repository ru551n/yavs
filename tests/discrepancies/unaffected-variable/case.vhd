-- LRM 2019 10.5.2.1: `unaffected` in a variable assignment leaves the target unchanged.
entity top is end entity;
architecture a of top is
begin
  process
    variable v : integer := 5;
    variable w : integer := 6;
    constant c : boolean := false;
  begin
    v := unaffected;
    w := 3 when c else unaffected;
    report "RES v=" & integer'image(v) & " w=" & integer'image(w);
    wait;
  end process;
end architecture;
