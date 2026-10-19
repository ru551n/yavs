-- difftest: top=force_in_port std=2008 ref=hand
-- A force of an `in` port acts on the port only, never on its actual (LRM 10.5.2.1,
-- 14.7.3.3): from a process of the instance and through an external name. The unforced port
-- `c` reads its actual. ref=hand: the reference simulator also changes the actual (a port
-- collapsed into it, catalogue `force-in-port-actual`); the other two references agree with
-- this file, in this report order too.
entity force_in_port_child is
  port (a, b, c : in integer);
end;
architecture t of force_in_port_child is
begin
  process begin
    wait for 1 ns;
    a <= force 5;
    wait for 1 ns;
    report "child a=" & integer'image(a) & " b=" & integer'image(b) & " c=" & integer'image(c);
    a <= release;
    wait for 1 ns;
    report "child released a=" & integer'image(a) & " b=" & integer'image(b);
    wait;
  end process;
end;

entity force_in_port is end;
architecture t of force_in_port is
  component force_in_port_child is
    port (a, b, c : in integer);
  end component;
  signal s, t, u : integer := 1;
begin
  u1 : force_in_port_child port map (a => s, b => t, c => u);
  process begin
    wait for 1 ns;
    << signal .force_in_port.u1.b : integer >> <= force 7;
    u <= 3;
    wait for 1 ns;
    report "top s=" & integer'image(s) & " t=" & integer'image(t) & " u=" & integer'image(u);
    << signal .force_in_port.u1.b : integer >> <= release;
    wait for 1 ns;
    report "top released s=" & integer'image(s) & " t=" & integer'image(t);
    wait;
  end process;
end;
