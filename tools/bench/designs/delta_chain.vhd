-- Deep delta chain: a ring of zero-delay assignments; one input change ripples through DEPTH deltas.
entity bench_delta_chain is
  generic (DEPTH : positive := 400; ROUNDS : positive := 400);
end entity;

architecture sim of bench_delta_chain is
  signal c : bit_vector(0 to DEPTH) := (others => '0');
begin
  chain : for i in 0 to DEPTH - 1 generate
    c(i + 1) <= c(i);
  end generate;

  process
  begin
    for r in 1 to ROUNDS loop
      c(0) <= not c(0);
      wait on c(DEPTH);
      wait for 1 ns;
    end loop;
    report "delta_chain done";
    wait;
  end process;
end architecture;
