-- difftest: top=array_record_whole std=2008 ref=hand
-- ref=hand: the reference simulator crashes in the resolution function, and the second one
-- refuses the partial drivers. With process b driving the equivalent whole values instead,
-- the second reference simulator gives these values (LRM 14.7.2, as in array_whole).
-- Composites with records resolved as a whole (LRM 4.6, #200): an array of records and a
-- record with an array-of-records field. A process driving one element, one field of an
-- element or one bit drives the whole composite (LRM 14.7.2), the rest keeping the initial
-- value.
entity array_record_whole is end;
architecture t of array_record_whole is
  type cell is record ok : boolean; n : natural; b : bit_vector(1 downto 0); end record;
  type cell_vec is array (natural range <>) of cell;
  subtype cells is cell_vec(0 to 1);
  type cells_vec is array (natural range <>) of cells;
  function csum(v : cells_vec) return cells is
    variable r : cells := (others => (false, 0, "00"));
  begin
    for i in v'range loop
      for k in r'range loop
        r(k).ok := r(k).ok or v(i)(k).ok;
        r(k).n := r(k).n + v(i)(k).n;
        r(k).b := r(k).b xor v(i)(k).b;
      end loop;
    end loop;
    return r;
  end;
  subtype rcells is csum cells;

  type blk is record tag : character; cs : cells; end record;
  type blk_vec is array (natural range <>) of blk;
  function bmax(v : blk_vec) return blk is
    variable r : blk := (nul, (others => (false, 0, "00")));
  begin
    for i in v'range loop
      if v(i).tag > r.tag then r.tag := v(i).tag; end if;
      for k in r.cs'range loop
        r.cs(k).n := r.cs(k).n + v(i).cs(k).n;
        r.cs(k).ok := r.cs(k).ok or v(i).cs(k).ok;
        r.cs(k).b := r.cs(k).b or v(i).cs(k).b;
      end loop;
    end loop;
    return r;
  end;
  subtype rblk is bmax blk;

  signal rc : rcells := ((false, 1, "01"), (false, 2, "10"));
  signal rb : rblk := ('a', ((false, 1, "00"), (true, 0, "01")));

  function img(c : cell) return string is
  begin
    return boolean'image(c.ok) & "/" & integer'image(c.n) & "/" & bit'image(c.b(1))
      & bit'image(c.b(0));
  end;
  function img(v : cells) return string is
  begin
    return img(v(0)) & " " & img(v(1));
  end;
begin
  a : process begin
    rc <= ((true, 10, "11"), (false, 20, "00")) after 1 ns;
    rb <= ('c', ((false, 5, "10"), (false, 6, "00"))) after 1 ns;
    wait;
  end process;
  b : process
    variable k : natural := 1;
  begin
    wait for 2 ns;
    rc(k).n <= 7;
    rb.cs(0).b(1) <= '1';
    wait for 1 ns;
    rc(0) <= (true, 3, "11");
    rb.tag <= 'z';
    wait for 1 ns;
    assert rc = ((true, 13, "00"), (false, 27, "10")) severity failure;
    assert rb = ('z', ((false, 6, "10"), (true, 6, "01"))) severity failure;
    report "done";
    wait;
  end process;
  process (rc) begin report "rc " & img(rc); end process;
  process (rb) begin report "rb " & rb.tag & " " & img(rb.cs); end process;
end;
