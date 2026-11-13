-- difftest: top=serial_line_roots
-- Serial processes keep lines, objects of the shared heap, across many waits (one in a procedure
-- with a line of its own), and most wakes leave them as they were, while another serial process
-- allocates strings and frees half of them: every line survives the collections with what was
-- written to it. Only sums are printed, so the order the processes end in does not show.
use std.textio.all;
entity serial_line_roots is end;
architecture a of serial_line_roots is
  type tally_t is protected
    procedure add (n : integer);
    impure function get return integer;
  end protected;
  type tally_t is protected body
    variable t : integer := 0;
    procedure add (n : integer) is begin t := (t + n) mod 1000003; end;
    impure function get return integer is begin return t; end;
  end protected body;
  shared variable tally : tally_t;
  shared variable lens : tally_t;
  signal net : integer := -1;
  signal done : boolean := false;
begin
  g : for i in 0 to 3 generate
    process
      variable l : line;
      variable h : natural := 0;
      variable s : natural := 0;
      procedure await is
        variable m : line;
      begin
        write(m, i);
        wait on net, done until net = i or done;
        assert m.all = integer'image(i) severity failure;
        deallocate(m);
      end procedure;
    begin
      write(l, string'("p") & integer'image(i));
      loop
        if i mod 2 = 0 then
          wait on net, done until net = i or done;
        else
          await;
        end if;
        exit when done;
        h := h + 1;
        if h mod 7 = 0 then
          write(l, string'(":") & integer'image(h));
        end if;
      end loop;
      for k in l'range loop
        s := (s + character'pos(l(k)) * k) mod 1000003;
      end loop;
      tally.add(s);
      lens.add(l'length);
      wait;
    end process;
  end generate;
  churn : process
    type str_ptr is access string;
    variable p, q : str_ptr;
  begin
    for e in 0 to 299 loop
      tally.add(0);  -- serial: its strings are in the shared heap too
      p := new string'("garbage " & integer'image(e));
      deallocate(p);
      q := new string'(integer'image(e));
      net <= e mod 5;
      wait for 1 ns;
    end loop;
    done <= true;
    wait for 1 ns;
    report "sum " & integer'image(tally.get) & " length " & integer'image(lens.get);
    wait;
  end process;
end;
