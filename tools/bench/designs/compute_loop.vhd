-- Sequential compute: integer arithmetic, a function call, array updates, nested loops.
entity bench_compute_loop is
  generic (N : natural := 1000000);
end entity;

architecture sim of bench_compute_loop is
  type int_vec is array (0 to 255) of integer;

  function mix(a, b : integer) return integer is
  begin
    return (a * 31 + b * 17 + 7) mod 1000003;
  end function;
begin
  process
    variable v   : int_vec := (others => 1);
    variable acc : integer := 0;
    variable i   : integer;
  begin
    for k in 1 to N loop
      i := k mod 256;
      v(i) := mix(v(i), acc);
      acc := (acc + v((i * 7) mod 256)) mod 1000003;
    end loop;
    assert acc >= 0 report "unreachable" severity failure;
    report "compute_loop acc=" & integer'image(acc);
    wait;
  end process;
end architecture;
