-- difftest: top=port_expr_part_range_elem std=2008 exit=1
-- The equivalent process of an expression actual of an array element in an individual
-- association checks the value against the element subtype (LRM 6.5.6.3, 14.7.3.1, #263).
package port_expr_part_range_elem_pkg is
  type nv is array (natural range <>) of natural;
end package;
use work.port_expr_part_range_elem_pkg.all;
entity port_expr_part_range_elem_mon is port (v : in nv(0 to 1)); end;
architecture a of port_expr_part_range_elem_mon is
begin
  process (v) begin
    report "v=" & integer'image(v(0)) & "," & integer'image(v(1)) & " @" & time'image(now);
  end process;
end;
entity port_expr_part_range_elem is end;
architecture a of port_expr_part_range_elem is
  signal x : integer := 4;
begin
  m : entity work.port_expr_part_range_elem_mon port map (v(0) => x - 3, v(1) => 7);
  process begin
    wait for 1 ns;
    x <= 3;
    wait for 1 ns;
    x <= 2;
    wait;
  end process;
end;
