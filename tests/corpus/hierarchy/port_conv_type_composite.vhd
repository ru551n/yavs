-- difftest: top=port_conv_type_composite std=2008 stop=10ns
-- Type conversions of composite ports (LRM 6.5.7.1, 9.3.6, #74): arrays of records between
-- two array types of the same record, a record to its own type, and an array of integers
-- to an array of reals, in and out of the instance, in the delta of the source.
package port_conv_type_composite_pkg is
  type pair is record
    a : integer;
    b : bit_vector(1 downto 0);
  end record;
  type pairs is array (natural range <>) of pair;
  type pairs2 is array (natural range <>) of pair;
  type ints is array (natural range <>) of integer;
  type reals is array (natural range <>) of real;
end package;
use work.port_conv_type_composite_pkg.all;
entity port_conv_type_composite_sub is
  port (
    pi : in pairs(0 to 1);
    p : in pair;
    ri : in reals(1 to 2);
    po : out pairs(0 to 1)
  );
end;
architecture a of port_conv_type_composite_sub is
begin
  process (pi, p, ri) begin
    report "pi0=" & integer'image(pi(0).a) & "," & to_string(pi(0).b)
      & " pi1=" & integer'image(pi(1).a) & "," & to_string(pi(1).b)
      & " p=" & integer'image(p.a) & "," & to_string(p.b)
      & " ri=" & integer'image(integer(ri(1) * 10.0)) & "," & integer'image(integer(ri(2) * 10.0))
      & " @" & time'image(now);
    po <= (pi(1), pi(0));
  end process;
end;
use work.port_conv_type_composite_pkg.all;
entity port_conv_type_composite is end;
architecture a of port_conv_type_composite is
  signal s, t : pairs2(0 to 1) := ((1, "01"), (2, "10"));
  signal q : pair := (3, "11");
  signal n : ints(0 to 1) := (4, 5);
begin
  u : entity work.port_conv_type_composite_sub
    port map (pi => pairs(s), p => pair(q), ri => reals(n), pairs2(po) => t);
  process (t) begin
    report "t0=" & integer'image(t(0).a) & "," & to_string(t(0).b)
      & " t1=" & integer'image(t(1).a) & "," & to_string(t(1).b) & " @" & time'image(now);
  end process;
  process begin
    wait for 1 ns;
    s(1) <= (7, "00");
    q.a <= 8;
    wait for 1 ns;
    n(0) <= -6;
    wait;
  end process;
end;
