-- difftest: top=binary_file_composites std=2008
-- Files of records and of arrays of composites: write, close, reopen and read back. The
-- stored form (and the stored length of an array of composites) is implementation defined, so
-- only the values read back are compared (#79).
entity binary_file_composites is end;
architecture t of binary_file_composites is
  type rec is record
    a : integer;
    b : bit_vector(1 to 4);
    c : character;
    r : real;
  end record;
  type rec_array is array (natural range <>) of rec;
  type rec_f is file of rec;
  type rec_array_f is file of rec_array;
  type slv_array is array (natural range <>) of bit_vector(3 downto 0);
  type slv_array_f is file of slv_array;
begin
  process
    file fr : rec_f open write_mode is "binfile_rec.bin";
    file fa : rec_array_f open write_mode is "binfile_recarr.bin";
    file fs : slv_array_f open write_mode is "binfile_slvarr.bin";
    variable r1, r2 : rec;
    variable ra, rb : rec_array(1 to 2);
    variable s1, s2 : slv_array(0 to 2);
    variable len : natural;
  begin
    r1 := (a => 42, b => "1010", c => 'x', r => 2.5);
    write(fr, r1);
    r1 := (a => -1, b => "0001", c => 'y', r => -0.5);
    write(fr, r1);
    ra := ((1, "1111", 'a', 1.0), (2, "0000", 'b', 2.0));
    write(fa, ra);
    s1 := ("0001", "0010", "1000");
    write(fs, s1);
    file_close(fr); file_close(fa); file_close(fs);

    file_open(fr, "binfile_rec.bin", read_mode);
    read(fr, r2);
    assert r2 = (42, "1010", 'x', 2.5) report "first record" severity failure;
    assert not endfile(fr) severity failure;
    read(fr, r2);
    assert r2 = (-1, "0001", 'y', -0.5) report "second record" severity failure;
    assert endfile(fr) severity failure;
    file_close(fr);

    file_open(fa, "binfile_recarr.bin", read_mode);
    read(fa, rb, len);
    assert rb = ra and len = 2 report "array of records" severity failure;
    file_close(fa);

    file_open(fs, "binfile_slvarr.bin", read_mode);
    read(fs, s2, len);
    assert s2 = s1 report "array of vectors" severity failure;
    file_close(fs);
    report "binary composite files ok";
    wait;
  end process;
end;
