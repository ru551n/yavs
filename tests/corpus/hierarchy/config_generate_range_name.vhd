-- difftest: top=cfg_grn std=2008 ref=hand
-- The index specification of a block configuration is a discrete range: a range attribute or
-- a subtype name (LRM 1.3.1). The generate instances 0 to 2 take architecture `one`, 3 and 4
-- take architecture `two`. ref=hand: the first reference simulator gives `one` to all five (it
-- applies only the first block configuration); the second reference simulator agrees with the
-- LRM result in the .expected.
entity leaf_grn is generic (id : integer); end;
architecture one of leaf_grn is begin
  process begin report "leaf " & integer'image(id) & " one"; wait; end process;
end;
architecture two of leaf_grn is begin
  process begin report "leaf " & integer'image(id) & " two"; wait; end process;
end;

entity top_grn is end;
architecture t of top_grn is
  component leaf_grn is generic (id : integer); end component;
  subtype low_idx is integer range 0 to 2;
  type high_t is array (3 to 4) of bit;
begin
  g : for i in 0 to 4 generate
    u : component leaf_grn generic map (id => i);
  end generate;
end;

configuration cfg_grn of top_grn is
  for t
    for g(low_idx)
      for u : leaf_grn use entity work.leaf_grn(one); end for;
    end for;
    for g(high_t'range)
      for u : leaf_grn use entity work.leaf_grn(two); end for;
    end for;
  end for;
end;
