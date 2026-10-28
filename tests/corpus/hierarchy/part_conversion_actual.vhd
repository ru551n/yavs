-- difftest: top=part_conv std=2008
-- A conversion function or type conversion of a signal as the actual of a part of an `in`
-- port (`r.x => popcount(s)`, LRM 6.5.7.1) converts in the port association, as for a whole
-- port: the part starts at the converted value and follows the signal in the same delta
-- (#611). The expression actual `popcount(s) + 0` is the near-miss: a delta later.
entity part_conv is end entity;
architecture test of part_conv is
  type t_rec is record
    x : integer;
    y : bit;
    z : integer;
  end record;
  type iarr is array (0 to 2) of integer;
  function popcount (v : bit_vector) return natural is
    variable n : natural := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then
        n := n + 1;
      end if;
    end loop;
    return n;
  end function;
  signal s : bit_vector(1 to 3);
  signal t : bit;
  signal rl : real := 2.0;
begin
  b : block is
    port (r : t_rec; a : iarr);
    port map (r.x => popcount(s), r.y => t, r.z => popcount(s) + 0,
              a(0) => 7, a(1) => popcount(s), a(2) => integer(rl));
  begin
    process (r, a) is
    begin
      report "r.x=" & integer'image(r.x) & " r.z=" & integer'image(r.z)
        & " a=" & integer'image(a(0)) & "," & integer'image(a(1)) & "," & integer'image(a(2));
      assert r.x = popcount(s) and a(1) = popcount(s) and a(2) = integer(rl)
        report "converted part lags its signal" severity failure;
    end process;
  end block;
  stim : process is
  begin
    s <= "101";
    s <= transport "001" after 1 ns;
    rl <= transport 5.0 after 2 ns;
    wait;
  end process;
end architecture;
