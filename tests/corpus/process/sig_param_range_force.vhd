-- difftest: top=sig_param_range_force std=2008 exit=1 ref=hand
-- ref=hand: sim1 and sim2 accept the value (discrepancy `sig-param-range`). A force through a
-- signal formal and a conditional assignment to it: both values belong to the actual's subtype
-- (LRM 14.7.3, #1181).
entity sig_param_range_force is end entity;
architecture a of sig_param_range_force is
  signal s : integer range 0 to 9 := 0;
  procedure cond (signal o : out integer; c : boolean) is begin o <= 10 when c else 1; end procedure;
  procedure frc (signal o : out integer; v : integer) is begin o <= force v; end procedure;
begin
  process
  begin
    cond(s, false);
    wait for 1 ns;
    report "s=" & integer'image(s);
    frc(s, 6);
    wait for 1 ns;
    report "s=" & integer'image(s);
    frc(s, 12);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
