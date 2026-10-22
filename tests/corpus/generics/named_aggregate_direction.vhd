-- difftest: top=named_aggregate_direction
-- A named aggregate with generic-valued elements for a descending constrained array: its
-- direction is the context's (LRM 9.3.3.3), so choice i lands at index i, both in a
-- generate condition evaluated during elaboration and at run time (#513).
entity named_aggregate_direction is
  generic (g0 : boolean := true; g1 : boolean := true; g2 : boolean := false; g3 : boolean := false);
end entity;

architecture a of named_aggregate_direction is
  type en_t is array (3 downto 0) of boolean;
  constant up_c : en_t := (0 => g0, 1 => g1, 2 => g2, 3 => g3);
  constant down_c : en_t := (3 => g3, 2 => g2, 1 => g1, 0 => g0);
  constant range_c : en_t := (1 downto 0 => g0, 3 downto 2 => g3);
  type asc_t is array (0 to 3) of boolean;
  constant asc_c : asc_t := (3 => g3, 2 => g2, 1 => g1, 0 => g0);
  type m_t is array (1 downto 0, 0 to 1) of boolean;
  constant m_c : m_t := (0 => (0 => g0, 1 => g1), 1 => (1 => g3, 0 => g2));
  signal up, down, rng, asc : bit_vector(3 downto 0);
  signal m : bit_vector(3 downto 0);
begin
  g : for i in 0 to 3 generate
    u1 : if up_c(i) generate up(i) <= '1'; end generate;
    u0 : if not up_c(i) generate up(i) <= '0'; end generate;
    d1 : if down_c(i) generate down(i) <= '1'; end generate;
    d0 : if not down_c(i) generate down(i) <= '0'; end generate;
    r1 : if range_c(i) generate rng(i) <= '1'; end generate;
    r0 : if not range_c(i) generate rng(i) <= '0'; end generate;
    a1 : if asc_c(i) generate asc(i) <= '1'; end generate;
    a0 : if not asc_c(i) generate asc(i) <= '0'; end generate;
  end generate;

  gm : for r in 0 to 1 generate
    gc : for c in 0 to 1 generate
      m1 : if m_c(r, c) generate m(2 * r + c) <= '1'; end generate;
      m0 : if not m_c(r, c) generate m(2 * r + c) <= '0'; end generate;
    end generate;
  end generate;

  check : process
  begin
    wait for 1 ns;
    report "up " & to_string(up) & " down " & to_string(down) & " range " & to_string(rng)
      & " asc " & to_string(asc) & " m " & to_string(m);
    assert up = "0011" report "up" severity failure;
    assert down = "0011" report "down" severity failure;
    assert rng = "0011" report "range" severity failure;
    assert asc = "0011" report "asc" severity failure;
    assert m = "0011" report "m" severity failure;
    for i in 0 to 3 loop
      assert up_c(i) = (i < 2) and down_c(i) = (i < 2) and range_c(i) = (i < 2)
        and asc_c(i) = (i < 2) report "run-time value " & integer'image(i) severity failure;
    end loop;
    report "done";
    wait;
  end process;
end architecture;
