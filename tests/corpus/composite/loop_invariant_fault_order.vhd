-- difftest: top=loop_invariant_fault_order exit=1 ref=hand
-- ref=hand: the expected text is yavs's wording of the run-time error (the references fail on
-- the same index with other words).
-- The first pass of the inner loop fails twice over: `j` is outside `m(i)` (it changes in the
-- loop) and `i` is outside the rows of `r` (it does not). The read comes first, so its failure
-- is the one reported, also when the loop's invariant code runs before the loop (#526).
entity loop_invariant_fault_order is end;

architecture a of loop_invariant_fault_order is
  type mem_t is array (natural range <>) of bit_vector;
  type mat_t is array (natural range <>, natural range <>) of bit;

  function copy(m : mem_t; lo, hi : natural) return mat_t is
    variable r : mat_t(0 to 1, 0 to 7);
  begin
    for i in lo to hi loop
      for j in 7 downto 0 loop
        r(i, j) := m(i)(j);
      end loop;
    end loop;
    return r;
  end function;
begin
  process
    variable m : mem_t(0 to 3)(7 downto 0) := (others => "10100101");
    variable n : mem_t(0 to 3)(3 downto 0) := (others => "1010");
  begin
    assert copy(m, 0, 1)(1, 5) = '1' severity failure;
    report "first";
    assert copy(n, 2, 3)(0, 0) = '0' severity failure;
    report "not reached";
    wait;
  end process;
end;
