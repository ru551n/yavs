-- One process, 20 million iterations of integer modular arithmetic and real multiply/divide: the
-- pure sequential compute loop, no signals.
-- Head-to-head design (tools/bench/AGENTS.md); written for yavs.
entity bench_compute_h2h is
  generic (N : natural := 20000000);
end entity;
architecture a of bench_compute_h2h is
begin
  process
    variable x : integer := 1;
    variable acc : integer := 0;
    variable r : real := 1.0;
    variable s : real := 0.0;
  begin
    for i in 1 to N loop
      x := (x * 75 + 74) mod 65537;
      if x mod 3 = 0 then
        acc := (acc + x) mod 1000003;
      else
        acc := (acc * 3 + i) mod 1000003;
      end if;
      r := r * 1.0000001 + 0.5;
      if r > 1.0e6 then r := r / 1.0e6; end if;
      s := s + r / real(x + 1);
    end loop;
    report "x=" & integer'image(x) & " acc=" & integer'image(acc) & " s>0=" & boolean'image(s > 0.0);
    assert x /= 0 and acc >= 0 and s > 0.0 severity failure;
    wait;
  end process;
end architecture;
