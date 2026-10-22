-- difftest: top=record_signal_field_index_error std=2008 exit=1 ignore=fatalmsg
-- An out-of-range index in front of a field of a record signal fails as the index of any
-- array does (the message text is compared loosely: ignore=fatalmsg).
entity record_signal_field_index_error is
end entity;

architecture a of record_signal_field_index_error is
  type rec_t is record
    n : integer;
    v : bit_vector(3 downto 0);
  end record;
  type arr_t is array (0 to 2) of rec_t;
  signal arr : arr_t;
begin
  process
    variable i : integer := 1;
    variable x : integer;
  begin
    arr(1).n <= 4;
    wait for 1 ns;
    x := arr(i).n;
    assert x = 4 report "x" severity failure;
    i := 5;
    x := arr(i).n;
    report "not reached " & integer'image(x);
    wait;
  end process;
end architecture;
