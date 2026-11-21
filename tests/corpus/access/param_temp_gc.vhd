-- difftest: top=param_temp_gc std=2008 ref=hand
-- An access out or inout variable parameter that goes through a copy (the procedure, declared in
-- the process, names the actual) keeps the object it is given alive while the call allocates
-- enough to collect (#1189, #1187): out not assigned first, assigned first, and inout, in a
-- process and in a procedure.
-- sim1 passes access parameters by reference (`q` changes early); the .expected is hand-checked
-- against the LRM (4.2.2.2: access values by copy; sim2 agrees).
entity param_temp_gc is end entity;
architecture a of param_temp_gc is
  type int_ptr is access integer;
  type str_ptr is access string;
begin
  process
    variable q : int_ptr;
    procedure churn is
      variable junk : str_ptr;
    begin
      for k in 1 to 100000 loop
        junk := new string'("garbage garbage garbage garbage garbage garbage");
      end loop;
    end procedure;
    procedure mk (x : out int_ptr) is
      variable n : integer := 42;
    begin
      x := new integer'(n);
      churn;
      report "mk q null=" & boolean'image(q = null) & " x.all=" & integer'image(x.all);
    end procedure;
    procedure mk_first (x : out int_ptr) is
    begin
      x := new integer'(43);
      churn;
      report "mk_first q.all=" & integer'image(q.all) & " x.all=" & integer'image(x.all);
    end procedure;
    procedure acc (x : inout int_ptr) is
    begin
      x := new integer'(x.all + 1);
      churn;
      report "acc q.all=" & integer'image(q.all) & " x.all=" & integer'image(x.all);
    end procedure;
    -- The same in a procedure's frame, after a collection there.
    procedure wrap is
      variable lq : int_ptr;
      procedure mkw (x : out int_ptr) is
      begin
        x := new integer'(45);
        churn;
        report "mkw lq null=" & boolean'image(lq = null) & " x.all=" & integer'image(x.all);
      end procedure;
    begin
      churn;
      mkw(lq);
      report "after mkw lq.all=" & integer'image(lq.all);
    end procedure;
  begin
    mk(q);
    report "after mk q.all=" & integer'image(q.all);
    mk_first(q);
    report "after mk_first q.all=" & integer'image(q.all);
    acc(q);
    report "after acc q.all=" & integer'image(q.all);
    wrap;
    assert q.all = 44 report "wrong value after the calls" severity failure;
    wait;
  end process;
end architecture;
