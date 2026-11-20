-- LRM 5.5.2, 4.2.2.2: the implicit READ of a file type is a procedure with an out formal VALUE
-- of the element type; the value read is copied back to the actual, so it belongs to the
-- actual's subtype; 10 does not belong to `integer range 0 to 9` (#1175).
entity top is end entity;
architecture a of top is
  type ifile is file of integer;
begin
  process
    file f : ifile;
    variable q : integer range 0 to 9 := 0;
  begin
    file_open(f, "file-read-range.dat", write_mode);
    write(f, 10);
    file_close(f);
    file_open(f, "file-read-range.dat", read_mode);
    read(f, q);
    report "RES q=" & integer'image(q);
    wait;
  end process;
end architecture;
