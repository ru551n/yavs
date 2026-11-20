-- difftest: top=copy_back_range_protected std=2008 exit=1 ref=hand
-- ref=hand: sim1 copies the value back without a check (discrepancy `copy-back-range`); sim2
-- agrees with this output. An out formal of a protected method (LRM 4.2.2.2, #1175).
entity copy_back_range_protected is end entity;
architecture a of copy_back_range_protected is
  type pt is protected
    procedure get (variable o : out integer; v : integer);
  end protected;
  type pt is protected body
    procedure get (variable o : out integer; v : integer) is begin o := v; end procedure;
  end protected body;
  shared variable sv : pt;
begin
  process
    variable q : integer range 0 to 9 := 0;
  begin
    sv.get(q, 7);
    report "q=" & integer'image(q);
    sv.get(q, 10);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
