-- difftest: top=if_case_generate std=2008
entity unit is generic (mode : integer); end;
architecture a of unit is begin
  g : case mode generate
    when 0 => process begin report "mode zero"; wait; end process;
    when 1 | 2 => process begin report "mode one or two: " & integer'image(mode); wait; end process;
    when others => process begin report "other mode " & integer'image(mode); wait; end process;
  end generate;
  e : if mode > 1 generate
    process begin report "extra for " & integer'image(mode); wait; end process;
  else generate
    process begin report "plain " & integer'image(mode); wait; end process;
  end generate;
end;
entity if_case_generate is end;
architecture t of if_case_generate is
begin
  gg : for m in 0 to 3 generate u : entity work.unit generic map (m); end generate;
end;
