-- difftest: top=copy_back_range_state std=2008 exit=1 ref=hand
-- ref=hand: sim1 reads `k` again at each check (discrepancy `object-range-bound-variable`); sim2
-- agrees with this output. The actual is a protected state variable whose range is kept in the
-- object (#1176): the copy back checks against that range, not `k` (LRM 4.2.2.2, #1175).
entity copy_back_range_state is end entity;
architecture a of copy_back_range_state is
  procedure set (variable o : out integer; v : integer) is begin o := v; end procedure;
  type pt is protected
    procedure put (v : integer);
    procedure setk (v : integer);
    impure function get return integer;
  end protected;
  type pt is protected body
    variable k : integer := 9;
    variable q : integer range 0 to k := 0;
    procedure put (v : integer) is begin set(q, v); end procedure;
    procedure setk (v : integer) is begin k := v; end procedure;
    impure function get return integer is begin return q; end function;
  end protected body;
  shared variable sv : pt;
begin
  process
  begin
    sv.setk(3);
    sv.put(7);
    report "q=" & integer'image(sv.get);
    sv.put(10);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
