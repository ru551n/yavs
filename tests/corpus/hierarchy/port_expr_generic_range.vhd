-- difftest: top=port_expr_generic_range std=2008 exit=1
-- The equivalent process of an expression actual checks the value against the formal's
-- subtype, here a range given by the instance's generics (LRM 6.5.6.3, 14.7.3.1, #74).
entity port_expr_generic_range_mon is
  generic (lo, hi : integer);
  port (n : in integer range lo to hi := lo);
end;
architecture a of port_expr_generic_range_mon is
begin
  process (n) begin
    report "n=" & integer'image(n) & " @" & time'image(now);
  end process;
end;
entity port_expr_generic_range is end;
architecture a of port_expr_generic_range is
  signal x : integer := 1;
begin
  m : entity work.port_expr_generic_range_mon generic map (0, 3) port map (n => x * 2);
  process begin
    wait for 1 ns;
    x <= 2;
    wait;
  end process;
end;
