-- difftest: top=hand_edges std=2008 ref=hand
-- ref=hand: cases where the default reference simulator deviates from the LRM (#163).
-- Concatenation of two null arrays yields the right operand with its bounds (LRM 9.2.5);
-- a negative SLA/SRA count shifts the other way (LRM 9.2.3): `v sla -n` is `v sra n`.
entity hand_edges is end entity;

architecture test of hand_edges is
  function bounds(v : bit_vector) return string is
  begin
    return integer'image(v'left) & " " & integer'image(v'right);
  end;
begin
  process
    variable x : bit_vector(5 to 4);
    variable y : bit_vector(9 downto 10);
    constant v : bit_vector(0 to 3) := "1011";
  begin
    assert bounds(x & y) = "9 10" report "null & null: " & bounds(x & y) severity failure;
    assert (v sra 1) = "1101" report "sra 1" severity failure;
    assert (v sla 1) = "0111" report "sla 1" severity failure;
    assert (v sra (-1)) = "0111" report "sra -1" severity failure;
    assert (v sla (-1)) = "1101" report "sla -1" severity failure;
    report "v sra -1 = " & to_string(v sra (-1));
    report "v sla -1 = " & to_string(v sla (-1));
    report "null cat: " & bounds(x & y);
    wait;
  end process;
end architecture;
