-- difftest: top=array_whole std=2008 ref=hand
-- ref=hand: the reference simulator crashes on arrays resolved as a whole and gets record
-- fields of array type wrong. iv (fully driven) and nb match another simulator, which refuses
-- the partial drivers of iv(1) and pr.bits; their values follow LRM 14.7.2 as in
-- record_whole, which the reference simulator gets right.
-- Arrays resolved as a whole (LRM 4.6): an integer array, a bit_vector (logic planes) and a
-- record with an array field. A process that drives one element or one field drives the
-- whole composite (LRM 14.7.2), its other elements keeping the initial value.
entity array_whole is end;
architecture t of array_whole is
  type ivec is array (0 to 2) of integer;
  type ivec_vec is array (natural range <>) of ivec;
  function isum(v : ivec_vec) return ivec is
    variable r : ivec := (others => 0);
  begin
    for i in v'range loop
      for k in r'range loop r(k) := r(k) + v(i)(k) * (k + 1); end loop;
    end loop;
    return r;
  end;
  subtype rivec is isum ivec;

  subtype nib is bit_vector(3 downto 0);
  type nib_vec is array (natural range <>) of nib;
  function bxor(v : nib_vec) return nib is
    variable r : nib := "0000";
  begin
    for i in v'range loop r := r xor v(i); end loop;
    return r;
  end;
  subtype rnib is bxor nib;

  type pair is record tag : character; bits : bit_vector(0 to 2); end record;
  type pair_vec is array (natural range <>) of pair;
  function pmax(v : pair_vec) return pair is
    variable r : pair := (nul, "000");
  begin
    for i in v'range loop
      if v(i).tag > r.tag then r.tag := v(i).tag; end if;
      r.bits := r.bits or v(i).bits;
    end loop;
    return r;
  end;
  subtype rpair is pmax pair;

  signal iv : rivec := (1, 2, 3);
  signal nb : rnib := "0001";
  signal pr : rpair := ('a', "100");

  function img(v : ivec) return string is
  begin
    return integer'image(v(0)) & "," & integer'image(v(1)) & "," & integer'image(v(2));
  end;
  function img(v : bit_vector) return string is
    variable s : string(1 to v'length);
  begin
    for i in 0 to v'length - 1 loop
      if v'ascending then s(i + 1) := bit'image(v(v'left + i))(2);
      else s(i + 1) := bit'image(v(v'left - i))(2); end if;
    end loop;
    return s;
  end;
begin
  a : process begin
    iv <= (10, 20, 30) after 1 ns;
    nb <= "1100" after 1 ns;
    pr <= ('c', "001") after 1 ns;
    wait;
  end process;
  b : process begin
    wait for 2 ns;
    iv(1) <= 7;
    nb <= "0110";
    pr.bits <= "010";
    wait for 1 ns;
    pr.tag <= 'z';
    wait for 1 ns;
    assert iv = (11, 54, 99) and nb = "1010" and pr = ('z', "011") severity failure;
    report "done";
    wait;
  end process;
  process (iv) begin report "iv " & img(iv); end process;
  process (nb) begin report "nb " & img(nb); end process;
  process (pr) begin report "pr " & pr.tag & " " & img(pr.bits); end process;
end;
