-- difftest: top=loop_invariant_replay exit=1 ref=hand
-- ref=hand: the expected text is yavs's wording of the run-time error (the references fail on
-- the same index with other words).
-- The inner loop reads `v(j)` (`j` changes in the loop) before `w(k)`, `w(k + 1)` and
-- `w(k + 2)` (`k` does not). On the last call both `j` and `k` are out of range on the first
-- pass: the failure reported is the one of `v(j)`, also when the reads of `w` run before the
-- loop (#526).
entity loop_invariant_replay is end;

architecture a of loop_invariant_replay is
  type ints is array (integer range <>) of integer;

  function sums(v, w : ints; lo, hi, k : integer) return integer is
    variable s : integer := 0;
  begin
    for i in 1 to 2 loop
      for j in lo to hi loop
        s := s + v(j) + w(k) + w(k + 1) + w(k + 2) * i;
      end loop;
    end loop;
    return s;
  end function;
begin
  process
    variable v : ints(1 to 4) := (1, 2, 3, 4);
    variable w : ints(-3 to 2) := (10, 20, 30, 40, 50, 60);
  begin
    report "sums " & integer'image(sums(v, w, 1, 4, -3));
    report "sums " & integer'image(sums(v, w, 2, 3, 0));
    report "sums " & integer'image(sums(v, w, 4, 1, 99));
    report "sums " & integer'image(sums(v, w, 0, 3, 10));
    report "not reached";
    wait;
  end process;
end;
