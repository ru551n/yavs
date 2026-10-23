-- A null waveform element on a bus record signal resolved as a whole (LRM 10.5.2.2, 6.4.2.3):
-- the driver of every field disconnects, so the resolution function sees no source.
entity nullrec is end entity;

architecture test of nullrec is
  type pair is record a : bit; b : integer; end record;
  type pairs is array (natural range <>) of pair;
  function res(v : pairs) return pair is
    variable r : pair := (a => '0', b => -1);
  begin
    for i in v'range loop r := (a => r.a or v(i).a, b => r.b + v(i).b + 1); end loop;
    return r;
  end function;
  subtype rpair is res pair;
  signal n : rpair bus := ('0', 0);
begin
  process
  begin
    n <= ('1', 5) after 1 ns, null after 3 ns;
    wait for 2 ns;
    report "RES 2 ns " & bit'image(n.a) & integer'image(n.b);
    wait for 2 ns;
    report "RES 4 ns " & bit'image(n.a) & integer'image(n.b);
    n <= ('1', 7);
    wait for 1 ns;
    report "RES 5 ns " & bit'image(n.a) & integer'image(n.b);
    n <= null;
    wait for 1 ns;
    report "RES 6 ns " & bit'image(n.a) & integer'image(n.b);
    wait;
  end process;
end architecture;
