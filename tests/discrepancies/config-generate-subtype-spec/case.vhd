-- A block configuration for a for-generate whose index specification is a subtype
-- indication (LRM 3.4.2: a discrete range), `for g(integer range 1 to 2)`.
entity leaf is generic (id : integer); end;
architecture a1 of leaf is
begin
  process begin report "RES a1 " & integer'image(id); wait; end process;
end;
architecture a2 of leaf is
begin
  process begin report "RES a2 " & integer'image(id); wait; end process;
end;
entity top is end;
architecture t of top is
  component leaf is generic (id : integer); end component;
begin
  g : for i in 1 to 3 generate
    u : leaf generic map (id => i);
  end generate;
end;
configuration cfg of top is
  for t
    for g(integer range 1 to 2)
      for u : leaf use entity work.leaf(a1); end for;
    end for;
  end for;
end;
