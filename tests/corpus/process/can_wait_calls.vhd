-- difftest: top=can_wait_calls std=2008
-- Procedures that can wait but mostly do not (the wait sits behind a condition): chains of
-- them, `out`/`inout` copy-back before and after a wait that is taken, signal parameters
-- driven and waited on before and after it, a timed `wait until` with a timeout, recursion
-- that waits at its bottom, a protected object called around the wait, an access value held
-- across the wait while other processes allocate, and the call from a process with copies.
entity can_wait_calls is
  generic (g : integer := 3);
end entity;

architecture sim of can_wait_calls is
  type counter is protected
    procedure add(n : integer);
    impure function total return integer;
  end protected;
  type counter is protected body
    variable t : integer := 0;
    procedure add(n : integer) is
    begin
      t := t + n;
    end procedure;
    impure function total return integer is
    begin
      return t;
    end function;
  end protected body;
  shared variable cnt : counter;

  type node;
  type node_ptr is access node;
  type node is record
    v    : integer;
    nxt  : node_ptr;
  end record;

  signal clk  : bit := '0';
  signal busy : bit := '0';
  signal req  : integer := 0;
  signal ack  : integer := 0;

  -- The leaf: work on locals, then a wait only when asked.
  procedure leaf(variable r : inout integer; go : boolean; variable o : out integer) is
    variable l1 : integer := 1;
    variable l2 : integer := 2;
  begin
    for i in 1 to 4 loop
      l1 := (l1 * 7 + i) mod 1009;
      l2 := (l2 * 5 + r) mod 1013;
    end loop;
    o := l1 + l2;
    if go then
      wait for 1 ns;
      r := r + 1000;
    end if;
    r := (r + l1) mod 100003;
  end procedure;

  procedure mid(variable r : inout integer; go : boolean; variable o : out integer) is
    variable x : integer;
  begin
    leaf(r, go, x);
    o := x + 1;
  end procedure;

  procedure top(variable r : inout integer; go : boolean; variable o : out integer) is
  begin
    mid(r, go, o);
    cnt.add(o mod 7);
  end procedure;

  -- A signal parameter: driven before the wait, waited on, driven after.
  procedure handshake(signal rq : out integer; signal ak : in integer; n : integer; go : boolean) is
  begin
    rq <= n;
    if go then
      wait until ak = n for 20 ns;
      if ak /= n then
        report "timed out at " & integer'image(n);
      end if;
      rq <= n + 1;
    end if;
  end procedure;

  -- Recursion: only the bottom call may wait.
  procedure down(n : natural; variable acc : inout integer; go : boolean) is
  begin
    acc := acc + n;
    if n = 0 then
      if go then
        wait until clk = '1';
      end if;
    else
      down(n - 1, acc, go);
      acc := acc * 3 mod 10007;
    end if;
  end procedure;

  -- An access value in a variable across the wait.
  procedure keep(variable h : inout node_ptr; go : boolean) is
    variable p : node_ptr;
  begin
    p := new node'(h.v + 1, h);
    if go then
      wait for 2 ns;
    end if;
    h := p;
  end procedure;
begin
  clk <= not clk after 5 ns when now < 2 us;

  -- The other side of `handshake`: answers some requests and leaves the others to time out.
  responder : process (req)
  begin
    if req mod 3 /= 0 then
      ack <= req after 3 ns;
    end if;
  end process;

  -- Allocates and drops lists while `main` waits holding its own.
  churn : process
    variable h : node_ptr;
  begin
    for k in 1 to 200 loop
      h := null;
      for i in 1 to 50 loop
        h := new node'(i, h);
      end loop;
      wait for 1 ns;
    end loop;
    wait;
  end process;

  main : process
    variable r, o, acc, s : integer := 0;
    variable h : node_ptr;
  begin
    r := 1;
    for k in 1 to 300 loop
      top(r, k mod 37 = 0, o);
      s := (s + o) mod 100003;
    end loop;
    report "chain r=" & integer'image(r) & " s=" & integer'image(s) & " now=" & time'image(now);
    for k in 1 to 12 loop
      handshake(req, ack, k, k mod 2 = 0);
    end loop;
    wait for 30 ns;
    report "handshake req=" & integer'image(req) & " ack=" & integer'image(ack) & " now=" & time'image(now);
    for k in 1 to 20 loop
      acc := 0;
      down(k mod 6, acc, k mod 5 = 0);
      s := (s + acc) mod 100003;
    end loop;
    report "recursion s=" & integer'image(s) & " now=" & time'image(now);
    h := new node'(0, null);
    for k in 1 to 100 loop
      keep(h, k mod 10 = 0);
    end loop;
    o := 0;
    while h /= null loop
      o := o + h.v;
      h := h.nxt;
    end loop;
    report "list sum=" & integer'image(o) & " now=" & time'image(now);
    report "counter=" & integer'image(cnt.total);
    wait;
  end process;

  -- Instances of the generic make copies of this process; each one waits in the same leaf.
  copies : for i in 1 to 2 generate
    p : process
      variable r, o : integer := i;
    begin
      for k in 1 to 50 loop
        mid(r, k mod (g + i) = 0, o);
      end loop;
      report "copy " & integer'image(i) & " r=" & integer'image(r) & " now=" & time'image(now);
      wait;
    end process;
  end generate;
end architecture;
