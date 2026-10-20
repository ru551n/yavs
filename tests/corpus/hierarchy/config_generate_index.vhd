-- difftest: top=config_generate_index_cfg std=2008 ref=hand
-- ref=hand: LRM 3.4.2: the index of a block configuration for a for-generate is a static
-- expression; here it uses a generic of the configured entity, so it is evaluated in the
-- elaborated instance (g(2) takes architecture a1, the others the last one, a2).
entity leaf is generic (id : integer); end;
architecture a1 of leaf is
begin
  process begin report "a1 " & integer'image(id); wait; end process;
end;
architecture a2 of leaf is
begin
  process begin report "a2 " & integer'image(id); wait; end process;
end;
entity config_generate_index is generic (n : integer := 2); end;
architecture t of config_generate_index is
  component leaf is generic (id : integer); end component;
begin
  g : for i in 1 to 3 generate
    u : leaf generic map (id => i);
  end generate;
end;
configuration config_generate_index_cfg of config_generate_index is
  for t
    for g(n)
      for u : leaf use entity work.leaf(a1); end for;
    end for;
  end for;
end;
