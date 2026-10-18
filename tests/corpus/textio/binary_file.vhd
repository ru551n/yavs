-- difftest: top=binary_file std=2008
-- Files of non-text types: write, close, reopen and read back; one array per file, read
-- into an object of the same length (the stored length of longer reads is implementation
-- defined).
entity binary_file is end;
architecture t of binary_file is
  type color is (red, green, blue);
  type small is range -1 to 200;
  type int_vec is array (natural range <>) of integer;
  type int_f is file of integer;
  type real_f is file of real;
  type color_f is file of color;
  type char_f is file of character;
  type time_f is file of time;
  type small_f is file of small;
  type bool_f is file of boolean;
  type bv_f is file of bit_vector;
  type iv_f is file of int_vec;
begin
  process
    file fi : int_f open write_mode is "binfile_int.bin";
    file fr : real_f open write_mode is "binfile_real.bin";
    file fc : color_f open write_mode is "binfile_color.bin";
    file fch : char_f open write_mode is "binfile_char.bin";
    file ft : time_f open write_mode is "binfile_time.bin";
    file fs : small_f open write_mode is "binfile_small.bin";
    file fb : bool_f open write_mode is "binfile_bool.bin";
    file fbv : bv_f open write_mode is "binfile_bv.bin";
    file fiv : iv_f open write_mode is "binfile_iv.bin";
    variable i : integer;
    variable r : real;
    variable c : color;
    variable ch : character;
    variable t : time;
    variable s : small;
    variable b : boolean;
    variable bv : bit_vector(1 to 5);
    variable iv : int_vec(0 to 2);
    variable len : natural;
  begin
    write(fi, 42); write(fi, -7); write(fi, integer'low); write(fi, integer'high);
    write(fr, 1.5); write(fr, -0.25);
    write(fc, blue); write(fc, red);
    write(fch, 'a'); write(fch, 'Z');
    write(ft, 5 ns); write(ft, 1 sec);
    write(fs, -1); write(fs, 200);
    write(fb, true); write(fb, false);
    write(fbv, bit_vector'("10110"));
    write(fiv, int_vec'(7, -8, 9));
    file_close(fi); file_close(fr); file_close(fc); file_close(fch); file_close(ft);
    file_close(fs); file_close(fb); file_close(fbv); file_close(fiv);

    file_open(fi, "binfile_int.bin", read_mode);
    read(fi, i); assert i = 42 severity failure;
    read(fi, i); assert i = -7 severity failure;
    read(fi, i); assert i = integer'low severity failure;
    assert not endfile(fi) severity failure;
    read(fi, i); assert i = integer'high severity failure;
    assert endfile(fi) severity failure;
    file_close(fi);

    file_open(fr, "binfile_real.bin", read_mode);
    read(fr, r); assert r = 1.5 severity failure;
    read(fr, r); assert r = -0.25 severity failure;
    assert endfile(fr) severity failure;
    file_close(fr);

    file_open(fc, "binfile_color.bin", read_mode);
    read(fc, c); assert c = blue severity failure;
    read(fc, c); assert c = red severity failure;
    file_close(fc);

    file_open(fch, "binfile_char.bin", read_mode);
    read(fch, ch); assert ch = 'a' severity failure;
    read(fch, ch); assert ch = 'Z' severity failure;
    file_close(fch);

    file_open(ft, "binfile_time.bin", read_mode);
    read(ft, t); assert t = 5 ns severity failure;
    read(ft, t); assert t = 1 sec severity failure;
    file_close(ft);

    file_open(fs, "binfile_small.bin", read_mode);
    read(fs, s); assert s = -1 severity failure;
    read(fs, s); assert s = 200 severity failure;
    file_close(fs);

    file_open(fb, "binfile_bool.bin", read_mode);
    read(fb, b); assert b severity failure;
    read(fb, b); assert not b severity failure;
    file_close(fb);

    file_open(fbv, "binfile_bv.bin", read_mode);
    read(fbv, bv, len);
    assert len = 5 and bv = "10110" severity failure;
    assert endfile(fbv) severity failure;
    file_close(fbv);

    file_open(fiv, "binfile_iv.bin", read_mode);
    read(fiv, iv, len);
    assert len = 3 and iv(0) = 7 and iv(1) = -8 and iv(2) = 9 severity failure;
    file_close(fiv);
    report "binary files ok";
    wait;
  end process;
end;
