-- difftest: top=for_generate std=2008
entity for_generate is end;
architecture t of for_generate is
  signal s : bit_vector(0 to 3) := "0000";
begin
  g : for i in s'range generate
    s(i) <= '1' after (i + 1) * 1 ns;
    process (s(i)) begin report "bit " & integer'image(i) & " = " & bit'image(s(i)) & " @" & time'image(now); end process;
  end generate;
end;
