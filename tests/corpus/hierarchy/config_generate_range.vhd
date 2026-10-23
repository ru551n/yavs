-- difftest: top=config_generate_range_cfg std=2008 ref=hand
-- ref=hand: LRM 3.4.2: the index specification of a block configuration for a for-generate is a
-- discrete range, so a subtype indication is legal (g(1) and g(2) take a1); sim1
-- does not parse it, sim2 gives this result. A plain range (g(4)) and an index (g(3)) take a1,
-- g(5) keeps the last architecture, a2.
entity leaf is generic (id : integer); end;
architecture a1 of leaf is
begin
  process begin report "a1 " & integer'image(id); wait; end process;
end;
architecture a2 of leaf is
begin
  process begin report "a2 " & integer'image(id); wait; end process;
end;
entity config_generate_range is end;
architecture t of config_generate_range is
  component leaf is generic (id : integer); end component;
begin
  g : for i in 1 to 5 generate
    u : leaf generic map (id => i);
  end generate;
end;
configuration config_generate_range_cfg of config_generate_range is
  for t
    for g(integer range 1 to 2)
      for u : leaf use entity work.leaf(a1); end for;
    end for;
    for g(3)
      for u : leaf use entity work.leaf(a1); end for;
    end for;
    for g(4 to 4)
      for u : leaf use entity work.leaf(a1); end for;
    end for;
  end for;
end;
