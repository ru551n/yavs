-- difftest: top=cfg_cbc std=2008
-- A component declared in a block is configured by a component configuration inside the
-- block configuration of that block.
entity leaf_cbc is end;
architecture a of leaf_cbc is begin
  process begin report "leaf a"; wait; end process;
end;

entity top_cbc is end;
architecture t of top_cbc is
begin
  k : block
    component leaf1 is end component;
  begin
    u : component leaf1;
  end block;
end;

configuration cfg_cbc of top_cbc is
  for t
    for k
      for others : leaf1 use entity work.leaf_cbc(a);
      end for;
    end for;
  end for;
end;
