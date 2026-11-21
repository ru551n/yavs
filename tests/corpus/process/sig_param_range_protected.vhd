-- difftest: top=sig_param_range_protected std=2008 exit=1 ref=hand
-- ref=hand: sim1 and sim2 accept the value (discrepancy `sig-param-range`). A method of a
-- protected type with a signal formal: the value driven through it belongs to the actual's
-- subtype (LRM 14.7.3, #1181).
entity sig_param_range_protected is end entity;
architecture a of sig_param_range_protected is
  signal s : integer range 0 to 9 := 0;
  type pt is protected
    procedure drv (signal o : out integer; v : integer);
  end protected;
  type pt is protected body
    procedure drv (signal o : out integer; v : integer) is begin o <= v; end procedure;
  end protected body;
  shared variable p : pt;
begin
  process
  begin
    p.drv(s, 4);
    wait for 1 ns;
    report "s=" & integer'image(s);
    p.drv(s, 10);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
