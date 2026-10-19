-- difftest: top=port_expr_record_array std=2008 stop=10ns ref=hand
-- A non-static expression actual of an in port of an array of records (LRM 6.5.6.3, #74):
-- the equivalent process drives every leaf of every element, one delta after the expression
-- changes. ref=hand: the reference simulator crashes in elaboration on this actual; the
-- expected output is yavs's, with the values and times of the second reference simulator
-- (which prints no deltas).
package port_expr_record_array_pkg is
  type pair is record
    a : integer;
    b : bit_vector(1 downto 0);
  end record;
  type pairs is array (natural range <>) of pair;
end package;
use work.port_expr_record_array_pkg.all;
entity port_expr_record_array_mon is
  generic (tag : string);
  port (p : in pair := (-1, "00"); ps : in pairs(0 to 1) := (others => (-2, "11")));
end;
architecture a of port_expr_record_array_mon is
begin
  process (p, ps) begin
    report tag & " p=" & integer'image(p.a) & "," & to_string(p.b)
      & " ps0=" & integer'image(ps(0).a) & "," & to_string(ps(0).b)
      & " ps1=" & integer'image(ps(1).a) & "," & to_string(ps(1).b) & " @" & time'image(now);
  end process;
end;
use work.port_expr_record_array_pkg.all;
entity port_expr_record_array is end;
architecture a of port_expr_record_array is
  signal x : integer := 1;
  signal r : pair := (5, "10");
begin
  m : entity work.port_expr_record_array_mon generic map ("m")
    port map (ps => ((r.a + x, r.b), (r.a - x, not r.b)));
  process begin
    wait for 1 ns;
    x <= 3;
    wait for 1 ns;
    r <= (7, "00");
    wait;
  end process;
end;
