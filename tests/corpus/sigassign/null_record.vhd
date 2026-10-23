-- difftest: top=null_record std=2008
-- A null waveform element on a record signal disconnects the driver of every field (LRM
-- 10.5.2.2, #352, #595): the guarded record signal, resolved as a whole, loses the source and
-- its resolution function sees no driver; a value after a null connects it again.
entity null_record is end entity;

architecture test of null_record is
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

  procedure show(tag : string; v : pair) is
  begin
    report tag & " " & bit'image(v.a) & " " & integer'image(v.b);
  end procedure;
begin
  process begin
    n <= ('1', 5) after 1 ns, null after 3 ns;
    wait for 2 ns;
    show("n2", n);
    wait for 2 ns;
    show("n4", n);
    n <= ('1', 7);
    wait for 1 ns;
    show("n5", n);
    n <= null;
    wait for 1 ns;
    show("n6", n);
    wait;
  end process;
end architecture;
