-- difftest: top=port_expr_part_range std=2008 exit=1
-- The equivalent process of an expression actual of a part in an individual association
-- checks the value against the part's subtype (LRM 6.5.6.3, 14.7.3.1, #263): here a record
-- field whose range comes from the generic of the package instance the instance is given.
package port_expr_part_range_gp is
  generic (hi : integer);
  type rec is record f : integer range 0 to hi; g : bit; end record;
end package;
entity port_expr_part_range_mon is
  generic (package p is new work.port_expr_part_range_gp generic map (<>));
  port (r : in p.rec);
end;
architecture a of port_expr_part_range_mon is
begin
  process (r) begin
    report "r.f=" & integer'image(r.f) & " r.g=" & bit'image(r.g) & " @" & time'image(now);
  end process;
end;
entity port_expr_part_range is end;
architecture a of port_expr_part_range is
  package p3 is new work.port_expr_part_range_gp generic map (3);
  signal x : integer := 1;
  signal b : bit := '0';
begin
  m : entity work.port_expr_part_range_mon generic map (p => p3)
    port map (r.f => x + 1, r.g => not b);
  process begin
    wait for 1 ns;
    x <= 2;
    wait for 1 ns;
    x <= 3;
    wait;
  end process;
end;
