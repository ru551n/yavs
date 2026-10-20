-- difftest: top=gc_random
-- Randomized heap churn inside one long activation (no wait in the loop), so the heap is
-- collected many times in the middle of statements: linked lists that are grown, rebuilt,
-- shortened and deallocated at random, function results (a record and an array of access
-- values) that live only in temporaries while further calls allocate, handles held only in
-- a callee's variables and parameters, and a protected stack in shared memory. A model in
-- integer arrays checks every list after each step; the checksums at the end are the
-- output. The first process also uses a protected stack in a shared variable (it runs
-- serially and allocates in the shared heap); the second allocates in its own heap.
entity gc_random is
end entity;

architecture a of gc_random is
  type node;
  type node_ptr is access node;
  type node is record
    v    : integer;
    link : node_ptr;
  end record;
  type ptr_vec is array (natural range <>) of node_ptr;
  type int_vec is array (natural range <>) of integer;
  type bundle is record
    head : node_ptr;
    arr  : ptr_vec(0 to 3);
    n    : integer;
  end record;

  type stack_t is protected
    procedure push(v : integer);
    impure function pop return integer;
    impure function depth return integer;
  end protected;

  type stack_t is protected body
    variable top : node_ptr;
    variable n   : integer := 0;
    procedure push(v : integer) is
    begin
      top := new node'(v, top);
      n := n + 1;
    end procedure;
    impure function pop return integer is
      variable p : node_ptr := top;
      variable v : integer;
    begin
      v := p.v;
      top := p.link;
      deallocate(p);
      n := n - 1;
      return v;
    end function;
    impure function depth return integer is
    begin
      return n;
    end function;
  end protected body;

  shared variable stack : stack_t;

  -- A small linear congruential generator (no overflow in INTEGER).
  procedure rnd(variable seed : inout integer; n : integer; variable r : out integer) is
  begin
    seed := (seed * 75 + 74) mod 65537;
    r := seed mod n;
  end procedure;

  -- A fresh list of `len` nodes `base, base + 1, ...`, built with the head only in this
  -- function's variables while every node is allocated.
  impure function build(len, base : integer) return node_ptr is
    variable head : node_ptr;
  begin
    for i in len - 1 downto 0 loop
      head := new node'(base + i, head);
    end loop;
    return head;
  end function;

  -- Sum and length of a list (access values are passed as variables).
  procedure walk(variable p : in node_ptr; variable s, n : out integer) is
    variable q : node_ptr := p;
  begin
    s := 0;
    n := 0;
    while q /= null loop
      s := s + q.v;
      n := n + 1;
      q := q.link;
    end loop;
  end procedure;

  -- A record of fresh lists, returned through temporaries.
  impure function mk_bundle(base : integer) return bundle is
    variable b : bundle;
  begin
    b.head := build(3, base);
    for i in b.arr'range loop
      b.arr(i) := build(i + 1, base + 10 * i);
    end loop;
    b.n := base;
    return b;
  end function;

  impure function mk_vec(base : integer) return ptr_vec is
    variable v : ptr_vec(0 to 2);
  begin
    for i in v'range loop
      v(i) := new node'(base + i, null);
    end loop;
    return v;
  end function;

  procedure push_front(variable l : inout node_ptr; v : integer) is
  begin
    l := new node'(v, l);
  end procedure;

  procedure drop_front(variable l : inout node_ptr; free : boolean) is
    variable p : node_ptr := l;
  begin
    if p /= null then
      l := p.link;
      if free then
        deallocate(p);
      end if;
    end if;
  end procedure;

  procedure churn(seed0 : integer; steps : integer; tag : string) is
    variable seed  : integer := seed0;
    variable lists : ptr_vec(0 to 15);
    variable sums  : int_vec(0 to 15) := (others => 0);
    variable lens  : int_vec(0 to 15) := (others => 0);
    variable k, op, v, total, x, s, n : integer := 0;
    variable b     : bundle;
    variable tmp   : node_ptr;
  begin
    for step in 1 to steps loop
      rnd(seed, 16, k);
      rnd(seed, 6, op);
      rnd(seed, 1000, v);
      case op is
        when 0 | 1 =>
          push_front(lists(k), v);
          sums(k) := sums(k) + v;
          lens(k) := lens(k) + 1;
        when 2 =>
          -- The old list becomes garbage.
          lists(k) := build(v mod 40, v);
          lens(k) := v mod 40;
          sums(k) := lens(k) * v + lens(k) * (lens(k) - 1) / 2;
        when 3 =>
          if lists(k) /= null then
            sums(k) := sums(k) - lists(k).v;
            lens(k) := lens(k) - 1;
          end if;
          drop_front(lists(k), v mod 2 = 0);
        when 4 =>
          -- Function results that hold handles live only in temporaries while the link
          -- calls allocate.
          x := mk_bundle(v).head.link.v + mk_bundle(v + 1).arr(2).link.link.v
               + mk_vec(v)(1).v + mk_vec(v + 2)(2).v + build(5, v).link.link.link.v;
          assert x = (v + 1) + (v + 23) + (v + 1) + (v + 4) + (v + 3)
            report tag & ": temporaries lost at step " & integer'image(step) severity failure;
          b := mk_bundle(v);
          walk(b.arr(3), s, n);
          total := total + s + n + b.n + b.head.v;
        when others =>
          tmp := build(v mod 50, 0);
          walk(tmp, s, n);
          total := total + n;
      end case;
      walk(lists(k), s, n);
      assert s = sums(k) and n = lens(k)
        report tag & ": list " & integer'image(k) & " corrupt at step " & integer'image(step)
        severity failure;
    end loop;
    for i in lists'range loop
      walk(lists(i), s, n);
      assert s = sums(i) and n = lens(i) report tag & ": final list corrupt" severity failure;
      total := total + s + n;
    end loop;
    report tag & ": total " & integer'image(total);
  end procedure;

begin
  serial : process
    variable t : integer := 0;
  begin
    churn(1, 8000, "serial");
    for i in 1 to 4000 loop
      stack.push(i);
      stack.push(i + 1);
      stack.push(stack.pop + stack.pop + 1);
      t := t + stack.depth;
    end loop;
    report "stack " & integer'image(t) & " " & integer'image(stack.pop);
    wait;
  end process;

  private : process
  begin
    wait for 1 ns;
    churn(7, 8000, "private");
    wait;
  end process;
end architecture;
