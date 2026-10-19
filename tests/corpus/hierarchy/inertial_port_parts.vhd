-- difftest: top=inertial_port_parts std=2008 stop=20ns
-- `inertial` on a sub-element of an individual association (LRM 6.5.6.3, 6.5.7.1): the part is
-- an expression actual even for a signal name and follows it one delta later, while the
-- parts associated with the plain signal follow in the same delta (#261).
package inertial_port_parts_pkg is
  type pair is record a, b : bit; end record;
end;
use work.inertial_port_parts_pkg.all;
entity inertial_port_parts_mon is port (i : in bit_vector(1 downto 0); r : in pair); end;
architecture a of inertial_port_parts_mon is
begin
  process (i, r) begin
    report "i=" & to_string(i) & " r.a=" & bit'image(r.a) & " r.b=" & bit'image(r.b)
      & " @" & time'image(now);
  end process;
end;
entity inertial_port_parts is end;
architecture a of inertial_port_parts is
  signal s, t : bit := '0';
begin
  s <= '1' after 5 ns, '0' after 10 ns;
  t <= '1' after 7 ns;
  m : entity work.inertial_port_parts_mon
    port map (i(1) => inertial s, i(0) => t, r.a => s, r.b => inertial t);
end;
