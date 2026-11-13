-- Serial processes that keep a line, a root in the shared heap, across waits on a broadcast
-- signal (two wait in a procedure with a line of its own): a wake that leaves the lines as they
-- were publishes the roots it published before without a scan (`shared_publishes`); one that
-- writes a line scans again.
use std.textio.all;
entity serial_roots is end;
architecture a of serial_roots is
  type ctr_t is protected
    procedure inc (n : integer);
    impure function get return integer;
  end protected;
  type ctr_t is protected body
    variable t : integer := 0;
    procedure inc (n : integer) is begin t := t + n; end;
    impure function get return integer is begin return t; end;
  end protected body;
  shared variable stats : ctr_t;
  signal net : integer := -1;
  signal done : boolean := false;
begin
  w : for i in 0 to 3 generate
    process
      variable l : line;
      variable h : natural := 0;
      procedure await is
        variable m : line;
      begin
        wait on net, done until net = i or done;
      end procedure;
    begin
      write(l, string'("w"));
      loop
        if i < 2 then
          wait on net, done until net = i or done;
        else
          await;
        end if;
        exit when done;
        h := h + 1;
        if h mod 10 = 0 then write(l, h); end if;
      end loop;
      stats.inc(l'length * (i + 1));
      wait;
    end process;
  end generate;
  process
  begin
    for e in 0 to 199 loop
      net <= e mod 5;
      wait for 1 ns;
    end loop;
    done <= true;
    wait for 1 ns;
    report "s=" & integer'image(stats.get);
    wait;
  end process;
end;
