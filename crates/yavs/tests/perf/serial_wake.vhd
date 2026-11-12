-- Serial processes (a protected call each, taken once at the end) on a clock: their activations
-- take the path of other processes (`serial_execs`) and publish no roots they cannot hold
-- (`shared_publishes`); so do waiters on a broadcast signal whose protected call is never taken.
entity serial_wake is end;
architecture a of serial_wake is
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
  constant CYCLES : natural := 100;
  signal clk : bit := '0';
  signal net : integer := -1;
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  g : for i in 0 to 7 generate
    signal q : bit := '0';
  begin
    process (clk)
      variable c : natural := 0;
    begin
      if clk = '1' then
        q <= not q;
        c := c + 1;
        if c = CYCLES then stats.inc(c * (i + 1)); end if;
      end if;
    end process;
  end generate;
  w : for i in 0 to 3 generate
    process
      variable h : natural := 0;
    begin
      loop
        wait on net until net = i;
        h := h + 1;
        if h = 1000 then stats.inc(1); end if;
      end loop;
    end process;
  end generate;
  process (clk)
    variable e : natural := 0;
  begin
    if clk = '1' then net <= e mod 7; e := e + 1; end if;
  end process;
  process
  begin
    wait for CYCLES * 10 ns;
    report "s=" & integer'image(stats.get);
    done <= true;
    wait;
  end process;
end;
