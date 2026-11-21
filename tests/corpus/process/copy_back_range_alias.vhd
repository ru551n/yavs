-- difftest: top=copy_back_range_alias std=2008 exit=1 ref=hand
-- ref=hand: sim1 copies the value back without a check (discrepancy `copy-back-range`); sim2
-- agrees with this output. Another parameter of the call is the record that holds the actual:
-- while the out formal holds 10 the record still has the value from before the call, which is
-- checked after it (LRM 4.2.2.2, #1175).
entity copy_back_range_alias is end entity;
architecture a of copy_back_range_alias is
  type rec is record
    f : integer range 0 to 9;
    g : integer;
  end record;
  procedure set (variable o : out integer; v : integer; variable r : in rec) is
  begin
    o := v;
    report "r.f=" & integer'image(r.f);
  end procedure;
begin
  process
    variable x : rec := (5, 0);
  begin
    set(x.f, 7, x);
    report "x.f=" & integer'image(x.f);
    set(x.f, 10, x);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
