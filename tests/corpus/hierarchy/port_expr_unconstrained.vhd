-- difftest: top=port_expr_unconstrained std=2008 stop=10ns
-- Expression actuals of an unconstrained in port (LRM 6.5.6.3, #74): a static value gives
-- the port its bounds; a non-static expression needs a globally static subtype, here a
-- qualified expression, and a conversion function of a signal its constrained result
-- subtype.
entity port_expr_unconstrained_mon is
  port (v : in bit_vector);
end;
architecture a of port_expr_unconstrained_mon is
begin
  process (v) begin
    report "v=" & to_string(v) & " left=" & integer'image(v'left) & " right=" & integer'image(v'right)
      & " @" & time'image(now);
  end process;
end;
entity port_expr_unconstrained is end;
architecture a of port_expr_unconstrained is
  signal b : bit_vector(3 downto 0) := "0011";
  subtype b4 is bit_vector(3 downto 0);
  function inv(x : bit_vector) return b4 is begin return not x; end;
begin
  i1 : entity work.port_expr_unconstrained_mon port map (v => b4'(not b));
  i2 : entity work.port_expr_unconstrained_mon port map (v => "0101");
  i3 : entity work.port_expr_unconstrained_mon port map (v => inv(b));
  process begin
    wait for 1 ns;
    b <= "1100";
    wait;
  end process;
end;
