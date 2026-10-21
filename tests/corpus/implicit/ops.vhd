-- difftest: top=ops std=2008
-- Implicit operations (LRM 2008 9.2): MINIMUM/MAXIMUM on scalars and arrays, shifts and
-- rotates on BIT_VECTOR, matching relational operators, the condition operator, array
-- concatenation and ordering, through sem, lowering and the interpreter (#163). Negative
-- shift amounts of sla/sra and null concatenation operands are left out: sim1 deviates there
-- (docs/decisions.md "Implicit operation natives").
entity ops is end entity;

architecture test of ops is
  type color is (red, green, blue);
begin
  process
    variable a : bit_vector(3 downto 0) := "1001";
    variable i : integer := 7;
    variable c : color := green;
  begin
    assert minimum(3, i) = 3 and maximum(3, i) = 7 severity failure;
    assert minimum(red, c) = red and maximum(blue, c) = blue severity failure;
    assert maximum(a) = '1' and minimum(a) = '0' severity failure;
    assert (a sll 1) = "0010" and (a srl 1) = "0100" severity failure;
    assert (a rol 1) = "0011" and (a ror 1) = "1100" severity failure;
    assert (a sla 1) = "0011" and (a sra 1) = "1100" severity failure;
    assert (a sll -1) = "0100" severity failure;
    assert (a ?= "1001") = '1' and (a ?/= "1001") = '0' severity failure;
    assert ('1' ?< '0') = '0' and ('0' ?<= '1') = '1' severity failure;
    assert ??('1') severity failure;
    assert (a & "01") = "100101" and ('1' & a) = "11001" severity failure;
    assert "0111" < a and a <= "1001" and not (a > "1001") severity failure;
    assert (not a) = "0110" and (a xor "1111") = "0110" severity failure;
    assert (and a) = '0' and (or a) = '1' and (xor a) = '0' severity failure;
    report to_string(a sll 2) & " " & to_string(a ror 3) & " " & to_string(maximum(i, -i));
    wait;
  end process;
end architecture;
