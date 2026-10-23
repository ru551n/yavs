-- difftest: top=alias_subtype_open_elements std=2019
-- An object alias whose subtype constrains only the index of an array of open arrays: the
-- alias slides the index range (0 to n-1) and keeps the element bounds of the object.
entity alias_subtype_open_elements is end entity;

architecture a of alias_subtype_open_elements is
  type barr is array (natural range <>) of bit_vector;

  function first (x : barr) return bit_vector is
    alias y : barr(0 to x'length - 1) is x;
  begin
    return y(0);
  end function;

  function last (x : barr) return bit_vector is
    alias y : barr(0 to x'length - 1) is x;
  begin
    return y(y'right);
  end function;

  function width (x : barr) return natural is
    alias y : barr(0 to x'length - 1) is x;
  begin
    return y(0)'length;
  end function;
begin
  process
    constant d : barr(1 to 3)(4 downto 1) := ("1010", "0110", "0001");
  begin
    assert first(d(2 to 3)) = "0110" report "first" severity failure;
    assert last(d(2 to 3)) = "0001" report "last" severity failure;
    assert width(d) = 4 report "width" severity failure;
    report "ok " & to_string(first(d)) & " " & to_string(last(d(1 to 2)));
    wait;
  end process;
end architecture;
