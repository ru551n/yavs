-- difftest: top=process_loop_invariant exit=1 ref=hand ignore=order
-- ref=hand: the expected text is yavs's wording of the run-time error (the references fail on
-- the same index with other words). ignore=order: the two instances report in the same cycles.
-- Loops of processes read instance constants (an array made from a generic) on every pass. A
-- loop without a wait may compute their addresses once before it, a loop with a wait may not,
-- and an invariant read that fails must fail after the effects of the passes before it (#541).
entity process_loop_invariant_sub is
  generic (taps : integer_vector; n : natural; late : time; tag : string);
end;

architecture a of process_loop_invariant_sub is
  constant t : integer_vector(0 to taps'length - 1) := taps;
  signal clk : bit := '0';
begin
  clk <= not clk after 5 ns when now < 40 ns;

  process (clk)
    variable acc : integer := 0;
  begin
    if clk = '1' then
      for i in t'range loop
        if t(i) /= 0 then
          acc := acc + t(i) * (i + 1);
        end if;
      end loop;
      for i in t'high downto 0 loop
        for j in 0 to i loop
          acc := acc + t(j) - t(i);
        end loop;
      end loop;
      report tag & " acc " & integer'image(acc);
    end if;
  end process;

  process
    variable s : integer := 0;
  begin
    for i in t'range loop
      wait until clk = '1';
      s := s + t(i);
      report tag & " s " & integer'image(s);
    end loop;
    wait;
  end process;

  process
    variable x : integer := 0;
  begin
    wait for late;
    for k in 0 to 2 loop
      report tag & " pass " & integer'image(k);
      x := x + t(n);
    end loop;
    report tag & " x " & integer'image(x);
    wait;
  end process;
end;

entity process_loop_invariant is end;

architecture a of process_loop_invariant is
begin
  u1 : entity work.process_loop_invariant_sub generic map (taps => (3, 0, 5, 7), n => 2, late => 50 ns, tag => "u1");
  u2 : entity work.process_loop_invariant_sub generic map (taps => (1, 2, 0), n => 3, late => 60 ns, tag => "u2");
end;
