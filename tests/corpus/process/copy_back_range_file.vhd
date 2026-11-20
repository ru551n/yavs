-- difftest: top=copy_back_range_file std=2008 exit=1 ref=hand
-- ref=hand: sim1 and sim2 copy the value back without a check (discrepancy
-- `file-read-range`). The implicit READ of a file type is a procedure with an out formal
-- VALUE of the file's element type (LRM 5.5.2): the value read is copied back, so it belongs
-- to the subtype of the actual (LRM 4.2.2.2, #1175).
entity copy_back_range_file is end entity;
architecture a of copy_back_range_file is
  type ifile is file of integer;
begin
  process
    file f : ifile;
    variable q : integer range 0 to 9 := 0;
  begin
    file_open(f, "copy_back_range_file.dat", write_mode);
    write(f, 7);
    write(f, 10);
    file_close(f);
    file_open(f, "copy_back_range_file.dat", read_mode);
    read(f, q);
    report "q=" & integer'image(q);
    read(f, q);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
