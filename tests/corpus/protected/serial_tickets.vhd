-- difftest: top=serial_tickets
-- Serial processes sharing protected objects. Sixteen clocked processes take a ticket from one
-- protected counter in the same delta (read, then write back one more): which process gets which
-- ticket depends on the order they run in, so only what no order changes is printed (every ticket
-- taken once, their sum). A protected list holds access values that the processes push at the
-- rising edge and pop at the falling one while garbage piles up, so the shared heap is collected
-- between activations; a process calls a protected method only in a branch taken once, late;
-- another waits inside a procedure holding an access value of its own.
use std.textio.all;
entity serial_tickets is end;
architecture a of serial_tickets is
  type seen_t is array (0 to 1023) of boolean;
  type desk_t is protected
    impure function take return integer;
    procedure mark (t : integer);
    impure function count return integer;
  end protected;
  type desk_t is protected body
    variable n : integer := 0;
    variable seen : seen_t := (others => false);
    variable marks : integer := 0;
    impure function take return integer is
      variable t : integer;
    begin
      t := n;
      n := t + 1;
      return t;
    end;
    procedure mark (t : integer) is begin
      assert not seen(t) report "ticket " & integer'image(t) & " twice" severity failure;
      seen(t) := true;
      marks := marks + 1;
    end;
    impure function count return integer is begin return marks; end;
  end protected body;
  type list_t is protected
    procedure push (v : integer);
    impure function pop return integer;
    impure function size return integer;
  end protected;
  type list_t is protected body
    type node;
    type node_ptr is access node;
    type node is record
      v : integer;
      name : line;
      link : node_ptr;
    end record;
    variable head : node_ptr;
    variable len : integer := 0;
    variable junk : line;
    procedure push (v : integer) is begin
      head := new node'(v, new string'("n" & integer'image(v)), head);
      len := len + 1;
      for k in 1 to 16 loop
        junk := new string'("garbage " & integer'image(v * 10 + k));
      end loop;
    end;
    impure function pop return integer is
      variable p : node_ptr := head;
      variable v : integer;
    begin
      assert p /= null report "pop of an empty list" severity failure;
      assert p.name.all = "n" & integer'image(p.v) report "lost name" severity failure;
      v := p.v;
      head := p.link;
      deallocate(p.name);
      deallocate(p);
      len := len - 1;
      return v;
    end;
    impure function size return integer is begin return len; end;
  end protected body;
  shared variable desk : desk_t;
  shared variable list : list_t;
  constant CYCLES : natural := 40;
  signal clk : bit := '0';
  signal done : boolean := false;
  signal sums : integer_vector(0 to 15) := (others => 0);
  signal late : integer := 0;
  signal own : integer := 0;
begin
  clk <= not clk after 5 ns when not done;
  g : for i in 0 to 15 generate
    process (clk)
      variable s, t : integer := 0;
    begin
      if clk = '1' then
        t := desk.take;
        desk.mark(t);
        s := s + t;
        sums(i) <= s;
        if i mod 4 = 0 then list.push(t); end if;
      elsif i mod 4 = 1 and list.size > 8 then
        -- At the falling edge, apart from the pushes: four poppers bring the list back to 8.
        t := list.pop;
        assert t >= 0 report "popped " & integer'image(t) severity failure;
      end if;
    end process;
  end generate;
  -- The only protected call of this process is in a branch taken once, at the last falling edge.
  process (clk)
    variable c : natural := 0;
  begin
    if clk = '1' then
      c := c + 1;
    elsif c = CYCLES then
      late <= desk.count;
    end if;
  end process;
  -- A wait inside a procedure with an access value of the procedure's own, across activations
  -- in which the protected objects collect; it reads the list 2 ns after each falling edge,
  -- when no other process acts.
  process
    type ip is access integer;
    procedure hold (n : natural; r : out integer) is
      variable p : ip := new integer'(7);
    begin
      for k in 1 to n loop
        wait until clk = '0';
        wait for 2 ns;
        p.all := p.all + list.size;
      end loop;
      r := p.all;
      deallocate(p);
    end;
    variable r : integer;
  begin
    hold(CYCLES - 1, r);
    own <= r;
    wait;
  end process;
  process
    variable l : line;
    variable total : integer := 0;
  begin
    wait for CYCLES * 10 ns;
    done <= true;
    wait for 1 ns;
    for i in 0 to 15 loop total := total + sums(i); end loop;
    write(l, string'("tickets ") & integer'image(desk.count) & " sum " & integer'image(total));
    writeline(output, l);
    write(l, string'("late ") & integer'image(late) & " left " & integer'image(list.size));
    writeline(output, l);
    write(l, string'("own ") & integer'image(own));
    writeline(output, l);
    wait;
  end process;
end;
