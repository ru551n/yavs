-- difftest: top=external_port_inertial std=2008 stop=40ns
-- External names inside expression and `inertial` port actuals (LRM 6.5.6.3, 8.7): each is
-- driven by its equivalent process in the instantiating region, one delta behind.
entity src is end;
architecture a of src is
  signal x : bit := '0';
  signal n : integer := 3;
begin
  x <= '1' after 10 ns, '0' after 20 ns;
  n <= 5 after 15 ns;
end;
entity mon is
  port (i : in bit; k : in integer; c : in integer);
end;
architecture a of mon is
begin
  process (i, k) begin
    report "i=" & bit'image(i) & " k=" & integer'image(k) & " c=" & integer'image(c);
  end process;
end;
entity external_port_inertial is end;
architecture tb of external_port_inertial is
  constant base : integer := 100;
begin
  s : entity work.src;
  m : entity work.mon port map (
    i => inertial << signal .external_port_inertial.s.x : bit >>,
    k => << signal .external_port_inertial.s.n : integer >> * 2 + 1,
    c => base + 1);
end;
