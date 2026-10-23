-- difftest: top=file_ops_2019 std=2019
-- The 2019 file operations on a file of character: the function form of FILE_OPEN, FILE_SIZE,
-- FILE_POSITION from each origin (from the end it is the distance to the end), FILE_SEEK,
-- FILE_TRUNCATE and FILE_REWIND.
entity file_ops_2019 is end entity;

architecture a of file_ops_2019 is
  type char_file is file of character;
begin
  process
    file f : char_file;
    variable c : character;
  begin
    assert file_open(f, "file_ops_2019.bin", WRITE_MODE) = OPEN_OK;
    assert file_mode(f) = WRITE_MODE;
    write(f, 'a');
    write(f, 'b');
    write(f, 'c');
    write(f, 'd');
    assert file_position(f) = 4 report "position after writing" severity failure;
    assert file_position(f, FILE_ORIGIN_END) = 0 report "end distance at the end" severity failure;
    assert file_position(f, FILE_ORIGIN_CURRENT) = 0 report "current" severity failure;
    assert file_size(f) = 4 report "size" severity failure;
    file_close(f);
    assert file_state(f) = STATE_CLOSED;

    assert file_open(f, "file_ops_2019.bin", READ_WRITE_MODE) = OPEN_OK;
    read(f, c);
    assert c = 'a';
    assert file_position(f) = 1 report "position after a read" severity failure;
    assert file_position(f, FILE_ORIGIN_END) = 3 report "end distance" severity failure;
    file_seek(f, 2);
    read(f, c);
    assert c = 'c';
    file_seek(f, 1);
    write(f, 'X');
    file_rewind(f);
    read(f, c);
    read(f, c);
    assert c = 'X' report "overwritten" severity failure;
    file_truncate(f, 2);
    assert file_size(f) = 2 report "truncated" severity failure;
    file_seek(f, 2);
    assert endfile(f);
    file_close(f);
    report "done";
    wait;
  end process;
end architecture;
