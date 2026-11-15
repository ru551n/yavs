-- difftest: top=slice_target_bound_null std=2008 ref=hand
-- ref=hand: a slice target of a guarded signal whose bound is an impure call, with null
-- waveform elements: the name is evaluated once for the whole waveform (LRM 10.5.2.1), so the
-- call runs once per assignment (#1157). From the LRM alone: one reference simulator runs the
-- call once per use (6 to 9 times here), the other cannot run null elements. The values of the
-- disconnected parts are left out: which sources a partly disconnected composite resolved signal
-- still has is not what this test is about.
entity slice_target_bound_null is
end entity;

architecture sim of slice_target_bound_null is
  type iv is array (natural range <>) of integer;
  type ivs is array (natural range <>) of iv(0 to 3);
  function resv(x : ivs) return iv is
  begin
    if x'length = 0 then
      return (0 to 3 => -1);
    end if;
    return x(x'left);
  end function;
  subtype rv is resv iv(0 to 3);
  type rec is record
    a : integer;
    b : iv(0 to 1);
  end record;
  type ra is array (natural range <>) of rec;
  type ras is array (natural range <>) of ra(0 to 3);
  function resa(x : ras) return ra is
  begin
    if x'length = 0 then
      return (0 to 3 => (-1, (-1, -1)));
    end if;
    return x(x'left);
  end function;
  subtype rra is resa ra(0 to 3);
  signal g : rv bus := (others => 0);
  signal gr : rra bus := (others => (0, (0, 0)));
begin
  process is
    variable n : natural := 0;
    impure function t(k : natural) return natural is
    begin
      n := n + 1;
      return k;
    end function;
  begin
    n := 0; g(t(1) to 2) <= (5, 6), null after 1 ns, (7, 8) after 2 ns;
    report "array: " & integer'image(n);
    n := 0; gr(t(1) to 2) <= (others => (3, (4, 5))), null after 1 ns;
    report "array of records: " & integer'image(n);
    wait for 1500 ps;
    n := 0; gr(t(1) to 2) <= gr(0 to 1), null after 1 ns, (others => (6, (7, 8))) after 2 ns;
    report "array of records, plain value: " & integer'image(n);
    wait for 3 ns;
    wait;
  end process;
end architecture;
