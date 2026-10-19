-- difftest: top=signal_record_element_range std=2008
-- The range of a record element of a signal with a record constraint is a legal bound of a
-- later subtype of the same region: the bounds come from the constraint, not from the value.
entity signal_record_element_range is end entity;
architecture a of signal_record_element_range is
  type t_if is record
    cs    : bit;
    rdata : bit_vector;
    wdata : bit_vector;
  end record;
  signal if_1 : t_if(rdata(7 downto 0), wdata(0 to 2));
  signal r : bit_vector(if_1.rdata'range);
  signal w : bit_vector(if_1.wdata'reverse_range);
begin
  process begin
    assert r'length = 8 and r'left = 7 and r'right = 0 report "rdata range" severity failure;
    assert w'length = 3 and w'left = 2 and w'right = 0 report "wdata range" severity failure;
    report "r " & integer'image(r'length) & " w " & integer'image(w'length);
    wait;
  end process;
end architecture;
