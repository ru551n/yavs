-- 24 plain concurrent copies (b <= a), the input toggled 39 times: copies the kernel carries out
-- itself, with no process activation per stage.
entity copy_chain is end;
architecture a of copy_chain is
  signal c : bit_vector(0 to 24) := (others => '0');
begin
  g: for i in 1 to 24 generate
    c(i) <= c(i-1);
  end generate;
  process begin
    for k in 1 to 39 loop
      c(0) <= not c(0);
      wait on c(24);
    end loop;
    report "c=" & to_string(c);
    wait;
  end process;
end;
