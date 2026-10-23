-- difftest: top=guarded_record_rec2 std=2008
-- The reproducer of #595: a bus record signal with a record resolution function.
entity guarded_record_rec2 is end entity;
architecture test of guarded_record_rec2 is
  type pair is record a : bit; b : integer; end record;
  type pairs is array (natural range <>) of pair;
  function res(v : pairs) return pair is
    variable r : pair := (a => '0', b => 0);
  begin
    for i in v'range loop r := v(i); end loop;
    return r;
  end function;
  subtype rpair is res pair;
  signal p : rpair bus;
begin
  process begin
    p <= ('1', 5) after 1 ns;
    wait for 2 ns;
    report integer'image(p.b);
    wait;
  end process;
end architecture;
