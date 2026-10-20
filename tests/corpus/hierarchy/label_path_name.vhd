-- difftest: top=label_path_name ignore=order
-- (ignore=order: reports of different processes in one cycle.)
-- 'path_name of a block or generate label names that statement itself (LRM 16.2.5), also
-- also below a for generate and a nested block (#376).
entity label_path_name is end;
architecture a of label_path_name is
begin
  g : if true generate
    process
    begin
      report "g " & g'path_name;
      wait;
    end process;
  end generate;
  b : block is
  begin
    process
    begin
      report "b " & b'path_name;
      wait;
    end process;
    n : block is
    begin
      process
      begin
        report "n " & n'path_name;
        wait;
      end process;
    end block;
  end block;
  f : for i in 1 to 2 generate
    process
    begin
      report "f " & f'path_name;
      wait;
    end process;
    q : block is
    begin
      process
      begin
        report "q " & q'path_name;
        wait;
      end process;
    end block;
  end generate;
  process
  begin
    wait for 1 ns;
    report "top " & label_path_name'path_name;
    wait;
  end process;
end;
