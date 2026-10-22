-- difftest: top=alias_slice_call_constant std=2008
-- An alias of a slice bounded by a constant that is initialised by a function call (impure,
-- on a generic): the constant is a globally static primary, so the name is static.
entity alias_slice_call_constant is
end entity;

architecture a of alias_slice_call_constant is
  impure function width(x : natural) return natural is
  begin
    return x;
  end function;

  signal s : bit_vector(15 downto 0) := x"A5C3";
  constant w : positive := width(8);
  alias lo is s(w - 1 downto 0);
  alias top_bit is s(w - 1);
begin
  process
  begin
    wait for 1 ns;
    assert lo'length = 8 severity failure;
    assert lo = x"C3" severity failure;
    assert top_bit = '1' severity failure;
    s <= x"0F01";
    wait for 1 ns;
    assert lo = x"01" severity failure;
    assert top_bit = '0' severity failure;
    report "done";
    wait;
  end process;
end architecture;
