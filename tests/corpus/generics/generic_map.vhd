-- difftest: top=generic_map std=2008
entity cell is generic (w : positive := 4; name : string := "dflt"; delay : time := 1 ns);
  port (o : out bit_vector(w - 1 downto 0));
end;
architecture a of cell is begin
  o <= (others => '1') after delay;
  process begin report name & " w=" & integer'image(w) & " d=" & time'image(delay); wait; end process;
end;
entity generic_map is end;
architecture t of generic_map is
  signal a : bit_vector(3 downto 0);
  signal b : bit_vector(1 downto 0);
begin
  u1 : entity work.cell port map (a);
  u2 : entity work.cell generic map (w => 2, name => "two", delay => 3 ns) port map (b);
  process (a, b) begin report "a=" & to_string(a) & " b=" & to_string(b) & " @" & time'image(now); end process;
end;
