-- Zero-delay chains and a feedback pair: the number of delta cycles of a time step.
entity delta_ring is end;
architecture a of delta_ring is
  signal c : bit_vector(0 to 16) := (others => '0');
  signal x, y : bit := '0';
  signal n : natural := 0;
begin
  g: for i in 0 to 15 generate
    c(i + 1) <= c(i);
  end generate;
  x <= not y when n < 20 else x;
  y <= x;
  process (x) begin n <= n + 1; end process;
  process begin
    for r in 1 to 20 loop
      c(0) <= not c(0);
      wait on c(16);
      wait for 1 ns;
    end loop;
    report "n=" & integer'image(n) & " c=" & to_string(c);
    wait;
  end process;
end;
