-- 'path_name of a block label seen from a block nested in it and from the enclosing region
-- (LRM 16.2.5).
entity top is end;
architecture a of top is
begin
  b : block is
  begin
    n : block is
    begin
      process
      begin
        report "b = " & b'path_name;
        wait;
      end process;
    end block;
  end block;
  process
  begin
    wait for 1 ns;
    report "b = " & b'path_name;
    wait;
  end process;
end;
