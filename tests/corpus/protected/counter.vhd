-- difftest: top=counter std=2019
entity counter is end;
architecture t of counter is
  type counter_t is protected
    procedure incr(by : integer := 1);
    impure function get return integer;
  end protected;
  type counter_t is protected body
    variable n : integer := 0;
    procedure incr(by : integer := 1) is begin n := n + by; end;
    impure function get return integer is begin return n; end;
  end protected body;
  shared variable c : counter_t;
begin
  p1 : process begin
    for i in 1 to 3 loop c.incr; wait for 1 ns; end loop;
    wait;
  end process;
  p2 : process begin
    wait for 5 ns;
    c.incr(10);
    report "count=" & integer'image(c.get);
    wait;
  end process;
end;
