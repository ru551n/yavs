-- difftest: top=tb_slice_param_range std=2008
-- A signal assigned through a slice whose range is the 'range of a parameter (open or
-- constrained) or of a process constant with open bounds: the driver is the whole signal
-- when the bounds are not known without a value (the longest static prefix, LRM 10.5.2,
-- 14.7.2), and elaboration needs no value for the parameter either way.
entity tb_slice_param_range is end;
architecture t of tb_slice_param_range is
  type rec_t is record
    data : bit_vector(7 downto 0);
    flag : bit;
  end record;
  subtype word is bit_vector(3 downto 0);
  signal s : bit_vector(7 downto 0) := (others => '0');
  signal r : rec_t := (data => (others => '0'), flag => '0');
begin
  process
    procedure put(d : bit_vector) is
    begin
      s(d'range) <= d;
      r.data(d'range) <= d;
    end procedure;
    procedure put_word(d : word) is
    begin
      s(d'range) <= d;
    end procedure;
    variable v : bit_vector(3 downto 0) := "1010";
    function mk return bit_vector is
      variable r : bit_vector(3 downto 2) := "11";
    begin
      return r;
    end function;
    constant w : bit_vector := mk;
  begin
    put(v);
    wait for 1 ns;
    assert s = "00001010" and r.data = "00001010" report "put(v)" severity failure;
    s(w'range) <= (others => '1');
    wait for 1 ns;
    assert s = "00001110" report "w'range" severity failure;
    put_word("0101");
    wait for 1 ns;
    assert s = "00000101" report "put_word" severity failure;
    report "ok";
    wait;
  end process;
end;
