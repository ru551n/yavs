-- difftest: top=binary_file_bounds exit=1 ref=hand
-- ref=hand: the first reference simulator reads the stored bytes into whatever object it is
-- given. A record read from a file must have the bounds of the object it is read into; yavs
-- stores the descriptor with the value and fails otherwise (#79).
entity binary_file_bounds is
  generic (n : natural := 3; m : natural := 5);
end;
architecture t of binary_file_bounds is
  type rec3 is record
    s : string(1 to n);
  end record;
  type rec5 is record
    s : string(1 to m);
  end record;
  type rec3_f is file of rec3;
  type rec5_f is file of rec5;
begin
  process
    file f3 : rec3_f open write_mode is "binfile_bounds.bin";
    file f5 : rec5_f;
    variable a : rec3 := (s => "abc");
    variable b : rec5;
  begin
    write(f3, a);
    file_close(f3);
    file_open(f5, "binfile_bounds.bin", read_mode);
    report "reading";
    read(f5, b);
    report "not reached";
    wait;
  end process;
end;
