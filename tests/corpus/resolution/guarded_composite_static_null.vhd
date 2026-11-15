-- difftest: top=guarded_composite_static_null std=2008 ref=hand
-- ref=hand: a process p1 drives a slice of guarded composites resolved as a whole by static
-- names, with null waveform elements; p2 drives the whole signals (#1160). By the LRM p1 has
-- drivers for elements 1 and 2 only (14.7.2); yavs gives it a source over the whole composite,
-- whose other elements keep the initial value. When all of the elements p1 drives are
-- disconnected the source takes no part in the resolution, as a unit (LRM 14.7.3.2; the
-- spare elements are not judged: decisions.md "Guarded record signals"): at 1.5 ns the value
-- is p2's alone, at 3.5 ns p1's alone (with the initial value 1 in elements 0 and 3), at 4.5
-- ns the resolution of no source. Both reference simulators crash on it.
entity guarded_composite_static_null is
end entity;

architecture sim of guarded_composite_static_null is
  type iv is array (natural range <>) of integer;
  type ivs is array (natural range <>) of iv(0 to 3);
  function sumv(x : ivs) return iv is
    variable r : iv(0 to 3) := (others => 0);
  begin
    if x'length = 0 then
      return (0 to 3 => -1);
    end if;
    for i in x'range loop
      for j in 0 to 3 loop
        r(j) := r(j) + x(i)(j);
      end loop;
    end loop;
    return r;
  end function;
  subtype rv is sumv iv(0 to 3);
  type rec is record
    a : integer;
    b : iv(0 to 1);
  end record;
  type ra is array (natural range <>) of rec;
  type ras is array (natural range <>) of ra(0 to 3);
  function suma(x : ras) return ra is
    variable r : ra(0 to 3) := (others => (0, (0, 0)));
  begin
    if x'length = 0 then
      return (0 to 3 => (-1, (-1, -1)));
    end if;
    for i in x'range loop
      for j in 0 to 3 loop
        r(j).a := r(j).a + x(i)(j).a;
        r(j).b(1) := r(j).b(1) + x(i)(j).b(1);
      end loop;
    end loop;
    return r;
  end function;
  subtype rra is suma ra(0 to 3);
  signal g : rv bus := (others => 1);
  signal gr : rra bus := (others => (1, (1, 1)));
  function img(v : iv) return string is
  begin
    return integer'image(v(0)) & " " & integer'image(v(1)) & " " & integer'image(v(2)) & " " &
      integer'image(v(3));
  end function;
begin
  p1 : process is
  begin
    g(1 to 2) <= (5, 6), null after 1 ns, (7, 8) after 2 ns, null after 4 ns;
    gr(1 to 2) <= (others => (3, (4, 5))), null after 1 ns, (others => (6, (7, 8))) after 2 ns,
      null after 4 ns;
    wait;
  end process;
  p2 : process is
  begin
    g <= (10, 20, 30, 40), null after 3 ns;
    gr <= (others => (10, (0, 20))), null after 3 ns;
    wait;
  end process;
  check : process is
  begin
    for i in 0 to 4 loop
      wait for 500 ps;
      report "g = " & img(g) & ", gr.a = " & integer'image(gr(0).a) & " " &
        integer'image(gr(1).a) & " " & integer'image(gr(2).a) & " " & integer'image(gr(3).a) &
        ", gr(2).b(1) = " & integer'image(gr(2).b(1));
      wait for 500 ps;
    end loop;
    wait;
  end process;
end architecture;
