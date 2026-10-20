-- difftest: top=access_method_call_2019 std=2019
-- A method called through an access value to a protected type, with and without `.all`
-- (LRM 2019 6.4.2.5, 5.6.2).
entity access_method_call_2019 is end;
architecture a of access_method_call_2019 is
  type counter is protected
    procedure inc;
    impure function get return natural;
  end protected;
  type counter is protected body
    variable n : natural := 0;
    procedure inc is begin n := n + 1; end procedure;
    impure function get return natural is begin return n; end function;
  end protected body;
  type counter_ptr is access counter;
begin
  process
    variable p, q : counter_ptr;
  begin
    p := new counter;
    q := p;
    p.inc;
    q.all.inc;
    report "get = " & integer'image(p.get);
    report "all.get = " & integer'image(p.all.get);
    assert q.get = 2 report "aliases share the object" severity failure;
    wait;
  end process;
end;
