-- difftest: top=delta_chain std=2008
entity delta_chain is end;
architecture t of delta_chain is
  signal s : bit_vector(0 to 5) := (others => '0');
begin
  s(0) <= '1' after 1 ns;
  g : for i in 1 to 5 generate
    s(i) <= s(i - 1);
  end generate;
  mon : process (s) begin
    report "s=" & to_string(s) & " @" & time'image(now);
  end process;
end;
