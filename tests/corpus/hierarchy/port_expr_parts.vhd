-- difftest: top=port_expr_parts std=2008 stop=20ns
-- Individual association of an in port whose parts mix a signal name, a static value and a
-- non-static expression (LRM 6.5.6.3, 6.5.7.1): the static part is in the initial value, the
-- others follow their expression one delta later. An operator actual is an expression, not a
-- conversion function.
entity mon is
  port (v : in bit_vector(5 downto 0); r : in bit := '1'; n : in natural := 3);
end;
architecture a of mon is
begin
  process (v, r, n) begin
    report "v=" & to_string(v) & " r=" & bit'image(r) & " n=" & integer'image(n)
      & " @" & time'image(now);
  end process;
end;
entity port_expr_parts is end;
architecture a of port_expr_parts is
  signal b : bit_vector(1 downto 0) := "01";
  signal s : bit := '0';
  signal i : integer := 2;
begin
  m : entity work.mon port map (
    v(5 downto 4) => b,
    v(3 downto 2) => "10",
    v(1 downto 0) => not b,
    r => not s,
    n => i - 1);
  process begin
    wait for 1 ns;
    b <= "10";
    s <= '1';
    i <= 7;
    wait;
  end process;
end;
