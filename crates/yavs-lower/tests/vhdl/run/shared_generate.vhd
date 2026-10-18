-- difftest: top=fe std=2008
entity fe is end;
architecture t of fe is
  type counter_t is protected impure function next_val return integer; end protected;
  type counter_t is protected body
    variable n : integer := 0;
    impure function next_val return integer is begin n := n + 1; return n; end;
  end protected body;
  shared variable c : counter_t;
begin
  g: for i in 0 to 31 generate
    p : process begin report integer'image(i) & " got " & integer'image(c.next_val); wait; end process;
  end generate;
end;
