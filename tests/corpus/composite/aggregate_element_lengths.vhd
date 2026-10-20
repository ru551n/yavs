-- difftest: top=aggregate_element_lengths std=2008 exit=1 ref=hand
-- The elements of an aggregate of an array type with an unconstrained element type must have
-- one length (LRM 9.3.3.3). With locally static elements the aggregate used to be folded into a
-- constant of mixed lengths. ref=hand: the first reference simulator rejects it at analysis, the
-- second fails at run time; yavs fails at run time, when the aggregate is evaluated.
entity aggregate_element_lengths is end;
architecture t of aggregate_element_lengths is
  type bv_array is array (natural range <>) of bit_vector;
  type bv_matrix is array (natural range <>) of bv_array;
  procedure show(m : bv_matrix) is
  begin
    report "rows " & integer'image(m'length);
  end procedure;
begin
  process
  begin
    show((0 => (x"0", x"3"), 1 => (x"c", x"f")));
    show((0 => (x"0", x"3"), 1 => (x"c", x"f0")));
    report "unreachable";
    wait;
  end process;
end;
