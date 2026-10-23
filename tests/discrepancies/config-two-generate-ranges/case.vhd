-- Two block configurations of one for-generate statement with disjoint index specifications
-- (a subtype name and a range attribute): instances 0 to 2 take architecture one, 3 and 4 two.
entity leaf_grn is generic (id : integer); end;
architecture one of leaf_grn is begin
  process begin report "RES " & integer'image(id) & " one"; wait; end process;
end;
architecture two of leaf_grn is begin
  process begin report "RES " & integer'image(id) & " two"; wait; end process;
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
