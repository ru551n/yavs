-- difftest: top=scalar_param_copy std=2008 ref=hand
-- A scalar or access variable parameter of mode out or inout is passed by copy (LRM 4.2.2.2):
-- a body that also reaches the actual by another name sees its old value until the call
-- returns (#1187). Reached through the actual's own name (assigned first, on every path,
-- inout), a second local procedure or function, a declaration's initial value, a parameter
-- default, an alias, a variable of a subprogram, a formal passed on, a field of a record
-- parameter, a loop range, a case selector, an array in actual naming the same object, an
-- element with a non-static index, a dereference and an access value. sim1 passes some of
-- these by reference and sees the new value; the .expected is hand-checked against the LRM
-- (sim2 agrees).
entity scalar_param_copy is end entity;
architecture a of scalar_param_copy is
  type real_arr is array (natural range <>) of real;
  type int_ptr is access integer;
  type rec is record
    a, b : integer;
  end record;
  -- A field of a record parameter, to a procedure that names the record by its own name.
  procedure rec_outer (r : inout rec) is
    procedure inner (x : inout integer) is
    begin
      x := 75;
      report "rec inner r.a=" & integer'image(r.a);
    end procedure;
    procedure two (x, y : inout integer) is
    begin
      x := x + 1;
      y := y + 2;
    end procedure;
  begin
    inner(r.a);
    report "rec after inner r.a=" & integer'image(r.a);
    two(r.a, r.b);
    report "rec after two r.a=" & integer'image(r.a) & " r.b=" & integer'image(r.b);
  end procedure;
  -- Not declared in the process: it reaches `arr(0)` only through the array formal.
  procedure through_array (x : out real; arr : in real_arr) is
  begin
    x := 1.0;
    report "through_array arr(0)=" & integer'image(integer(arr(0)));
  end procedure;
  -- The body names only its formal: passed by reference, nothing else sees it.
  procedure plain (x : inout integer) is
  begin
    x := x + 1;
  end procedure;
begin
  process
    variable v : integer := 5;
    alias av : integer is v;
    variable r : real_arr(0 to 1) := (2.0, 3.0);
    variable i : natural := 1;
    variable p, q : int_ptr;
    variable rv : rec := (70, 71);
    impure function get_v return integer is
    begin
      return v;
    end function;
    procedure show (what : string) is
    begin
      report what & " v=" & integer'image(v);
    end procedure;
    procedure show_decl (what : string) is
      variable t : integer := v;
    begin
      report what & " v=" & integer'image(t);
    end procedure;
    procedure show_default (what : string; t : integer := v) is
    begin
      report what & " v=" & integer'image(t);
    end procedure;
    procedure first (x : out integer) is
    begin
      x := 1;
      report "first v=" & integer'image(v);
    end procedure;
    procedure paths (x : out integer; c : boolean) is
    begin
      if c then x := 2; else x := 3; end if;
      report "paths v=" & integer'image(v);
    end procedure;
    procedure incr (x : inout integer) is
    begin
      x := x + 10;
      report "incr v=" & integer'image(v);
    end procedure;
    procedure by_proc (x : inout integer) is
    begin
      x := 20;
      show("by_proc");
    end procedure;
    procedure by_func (x : out integer) is
    begin
      x := 21;
      report "by_func v=" & integer'image(get_v);
    end procedure;
    procedure by_decl (x : out integer) is
    begin
      x := 22;
      show_decl("by_decl");
    end procedure;
    procedure by_default (x : out integer) is
    begin
      x := 23;
      show_default("by_default");
    end procedure;
    procedure by_alias (x : out integer) is
    begin
      x := 24;
      report "by_alias v=" & integer'image(av);
    end procedure;
    procedure elem (x : out real) is
    begin
      x := 9.0;
      report "elem r(i)=" & integer'image(integer(r(i)));
    end procedure;
    procedure deref (x : inout integer) is
    begin
      x := 30;
      report "deref p.all=" & integer'image(p.all);
    end procedure;
    procedure acc (x : inout int_ptr) is
    begin
      x := new integer'(41);
      report "acc q.all=" & integer'image(q.all);
    end procedure;
    -- Named after the assignment only in a loop range or a case selector.
    procedure range_after (x : out integer) is
    begin
      x := 70;
      for k in v to v loop
        report "range_after k=" & integer'image(k);
      end loop;
    end procedure;
    procedure case_after (x : out integer) is
    begin
      x := 71;
      case v is
        when 70 => report "case_after old v";
        when others => report "case_after new v";
      end case;
    end procedure;
    -- Read only before the formal is assigned: the same value either way.
    procedure read_before (x : out integer) is
    begin
      if v = 71 then x := 72; else x := 0; end if;
    end procedure;
    -- A variable of a subprogram, and a formal passed on to a procedure that reads it.
    procedure outer is
      variable lv : integer := 50;
      procedure inner (x : inout integer) is
      begin
        x := 51;
        report "inner lv=" & integer'image(lv);
      end procedure;
    begin
      inner(lv);
      report "after inner lv=" & integer'image(lv);
    end procedure;
    procedure pass_on (y : inout integer) is
      procedure inner2 (x : inout integer) is
      begin
        x := 61;
        report "inner2 y=" & integer'image(y);
      end procedure;
    begin
      inner2(y);
      report "after inner2 y=" & integer'image(y);
    end procedure;
  begin
    first(v);
    show("after first");
    paths(v, true);
    show("after paths");
    incr(v);
    show("after incr");
    by_proc(v);
    show("after by_proc");
    by_func(v);
    show("after by_func");
    by_decl(v);
    show("after by_decl");
    by_default(v);
    show("after by_default");
    by_alias(v);
    show("after by_alias");
    through_array(r(0), r);
    report "after through_array r(0)=" & integer'image(integer(r(0)));
    elem(r(i));
    report "after elem r(i)=" & integer'image(integer(r(i)));
    p := new integer'(29);
    deref(p.all);
    report "after deref p.all=" & integer'image(p.all);
    q := new integer'(40);
    acc(q);
    report "after acc q.all=" & integer'image(q.all);
    outer;
    v := 60;
    pass_on(v);
    show("after pass_on");
    plain(v);
    show("after plain");
    range_after(v);
    case_after(v);
    read_before(v);
    show("after read_before");
    rec_outer(rv);
    assert v = 72 and rv.a = 76 and rv.b = 73 and r(0) = 1.0 and r(1) = 9.0 and p.all = 30 and q.all = 41
      report "wrong values after the calls" severity failure;
    wait;
  end process;
end architecture;
