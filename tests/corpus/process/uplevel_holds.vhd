-- difftest: top=uplevel_holds std=2008
-- Nested subprograms reaching the variables of the frames around them (static links 1 and 2) and
-- inout scalars passed by address: from a process, from a procedure that calls them (its frame
-- below theirs), and from a procedure that waits (its frame is the running one), with calls and
-- allocations between the accesses. Compiled code resolves those frames once per call.
entity uplevel_holds is end entity;

architecture a of uplevel_holds is
  type ints_t is array (natural range <>) of integer;
  type ptr_t is access ints_t;
  type rec_t is record
    a, b : integer;
    v : ints_t(0 to 3);
  end record;

  function pad (s : string; n : natural) return string is
    variable r : string(1 to s'length + n) := (others => '.');
  begin
    r(1 to s'length) := s;
    return r;
  end function;
begin
  p : process
    variable v, s, w, u : integer := 0;
    variable r : rec_t := (1, 2, (3, 4, 5, 6));
    variable arr : ints_t(0 to 7) := (others => 1);
    variable q : ptr_t;

    -- Two uses of x, three of the process frame, nothing else: a stable leaf.
    procedure step (x : inout integer) is
    begin
      x := x + 1;
      s := s + u;
    end procedure;

    -- Records and arrays of the process frame, by run-time index.
    procedure mix (i : natural; x : inout integer) is
    begin
      r.v(i mod 4) := r.v(i mod 4) + x;
      arr(i mod 8) := arr(i mod 8) + r.a;
      x := x + arr((i + 1) mod 8);
      r.b := (r.b + r.v((i + 2) mod 4)) mod 65521;
    end procedure;

    -- Calls and allocations between the accesses.
    procedure churn (n : natural; x : inout integer) is
    begin
      w := w + pad("ab", n mod 5)'length;
      q := new ints_t'(0 to n mod 3 => n);
      w := w + q(q'left) + x;
      deallocate(q);
      x := x + pad("c", n mod 7)'length;
      w := w mod 100003;
    end procedure;

    -- A procedure whose nested procedure reaches both frames above it.
    procedure outer (n : natural; acc : inout integer) is
      variable o : integer := n;
      variable oa : ints_t(0 to 3) := (others => n);
      procedure inner (k : natural) is
      begin
        o := o + k;
        oa(k mod 4) := oa(k mod 4) + o;
        s := s + o;
        w := w + oa((k + 1) mod 4);
      end procedure;
      procedure inner_calls (k : natural) is
      begin
        o := o + pad("xy", k mod 3)'length;
        q := new ints_t'(0 to 1 => k);
        s := s + q(1) + o;
        deallocate(q);
        o := o mod 1009;
      end procedure;
    begin
      for k in 0 to 3 loop
        inner(k);
        inner_calls(k);
      end loop;
      acc := acc + o + oa(0) + oa(3);
    end procedure;

    -- A procedure that waits: its nested procedures run while its frame is the running one.
    procedure waits (n : natural; acc : inout integer) is
      variable o : integer := n;
      variable oa : ints_t(0 to 3) := (others => 2);
      procedure leaf (k : natural) is
      begin
        o := o + k;
        oa(k mod 4) := oa(k mod 4) + o;
      end procedure;
      procedure busy (k : natural) is
      begin
        o := o + pad("z", k mod 4)'length;
        oa(k mod 4) := oa(k mod 4) + o;
        o := o mod 997;
      end procedure;
    begin
      for k in 0 to 5 loop
        leaf(k);
        busy(k);
      end loop;
      wait for 1 ns;
      leaf(n);
      acc := acc + o + oa(1) + oa(2);
    end procedure;

    variable acc : integer := 0;
  begin
    for i in 1 to 2000 loop
      u := i mod 7;
      step(v);
      v := v mod 1000;
      mix(i, acc);
      acc := acc mod 65521;
      churn(i, acc);
      acc := acc mod 65521;
    end loop;
    report "step v=" & integer'image(v) & " s=" & integer'image(s);
    report "mix acc=" & integer'image(acc) & " r.b=" & integer'image(r.b) & " arr(3)=" & integer'image(arr(3));
    report "churn w=" & integer'image(w);
    s := 0;
    w := 0;
    for i in 1 to 200 loop
      outer(i, acc);
      acc := acc mod 65521;
    end loop;
    report "outer acc=" & integer'image(acc) & " s=" & integer'image(s) & " w=" & integer'image(w);
    for i in 1 to 20 loop
      waits(i, acc);
      acc := acc mod 65521;
    end loop;
    report "waits acc=" & integer'image(acc);
    wait;
  end process;
end architecture;
