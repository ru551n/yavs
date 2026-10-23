-- difftest: top=guarded_record std=2008 ref=hand
-- ref=hand: sim1 does not disconnect a guarded driver at initialisation (as bus_guarded), so
-- its idle and one lines count the other block's driver; .expected is the LRM result.
-- A guarded (bus or register) record signal whose subtype has a record resolution function
-- is resolved as a whole (LRM 4.6, 6.4.2.3, #595): the guard applies to the composite, not to
-- each field. A guarded assignment disconnects through GUARD, a bus then sees no source and
-- a register keeps its value.
entity guarded_record is end entity;

architecture test of guarded_record is
  type pair is record a : bit; b : integer; end record;
  type pairs is array (natural range <>) of pair;
  function res(v : pairs) return pair is
    variable r : pair := (a => '0', b => -1);
  begin
    for i in v'range loop r := (a => r.a or v(i).a, b => r.b + v(i).b + 1); end loop;
    return r;
  end function;
  subtype rpair is res pair;
  signal p : rpair bus := ('0', 0);
  signal q : rpair register := ('0', 0);
    signal en1, en2 : boolean := false;

  procedure show(tag : string; v : pair) is
  begin
    report tag & " " & bit'image(v.a) & " " & integer'image(v.b);
  end procedure;
begin
  g1 : block (en1) begin
    p <= guarded ('1', 10);
    q <= guarded ('1', 20);
  end block;
  g2 : block (en2) begin
    p <= guarded ('0', 100);
  end block;

  process begin
    wait for 1 ns;
    show("idle", p);
    en1 <= true; wait for 1 ns;
    show("one", p); show("regone", q);
    en2 <= true; wait for 1 ns;
    show("both", p);
    en1 <= false; wait for 1 ns;
    show("two", p); show("regkeep", q);
    en2 <= false; wait for 1 ns;
    show("none", p); show("regkeep2", q);
    wait;
  end process;

end architecture;
