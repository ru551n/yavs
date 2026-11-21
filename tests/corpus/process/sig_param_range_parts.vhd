-- difftest: top=sig_param_range_parts std=2008 exit=1 ref=hand
-- ref=hand: sim1 and sim2 accept the value (discrepancy `sig-param-range`). An element or a
-- field as the actual: the value driven through the formal belongs to the element's or the
-- field's subtype (LRM 14.7.3, #1181).
entity sig_param_range_parts is end entity;
architecture a of sig_param_range_parts is
  subtype digit is integer range 0 to 9;
  type digits is array (0 to 3) of digit;
  type rec is record
    d : digit;
    i : integer;
  end record;
  signal v : digits := (others => 0);
  signal r : rec := (0, 0);
  procedure drv (signal o : out integer; x : integer) is begin o <= x; end procedure;
begin
  process
  begin
    drv(v(2), 9);
    drv(r.i, 100);
    drv(r.d, 5);
    wait for 1 ns;
    report "v(2)=" & integer'image(v(2)) & " r.i=" & integer'image(r.i) & " r.d=" & integer'image(r.d);
    drv(v(3), 10);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
