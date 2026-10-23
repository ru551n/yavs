-- difftest: top=process_label_subtype std=2008
-- An expanded name with the label of the enclosing process as prefix denotes a type or subtype
-- of the process: a type mark in a declaration and a loop range (LRM 8.3).
entity process_label_subtype is end entity;

architecture a of process_label_subtype is
begin
  p : process
    subtype st is natural range 1 to 3;
    variable w : p.st := 2;
    variable total : integer := 0;
  begin
    for k in p.st loop
      total := total + k;
    end loop;
    assert total = 6 report "sum" severity failure;
    assert w = 2 and p.st'high = 3 report "types" severity failure;
    report "ok " & integer'image(total);
    wait;
  end process;
end architecture;
