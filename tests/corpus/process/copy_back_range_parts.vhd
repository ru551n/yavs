-- difftest: top=copy_back_range_parts std=2008 exit=1 ref=hand
-- ref=hand: sim1 copies the value back without a check (discrepancy `copy-back-range`); sim2
-- agrees with this output. Record fields and array elements as actuals are checked against
-- their own subtypes when copied back (LRM 4.2.2.2, #1175): values in range go back, then
-- 20 ns does not fit the physical field.
entity copy_back_range_parts is end entity;
architecture a of copy_back_range_parts is
  subtype short is time range 0 ns to 10 ns;
  subtype unit is real range 0.0 to 1.0;
  type rec is record
    t : short;
    f : integer range 0 to 9;
  end record;
  type rvec is array (0 to 3) of unit;
  procedure set_t (variable o : out time; v : time) is begin o := v; end procedure;
  procedure set_r (variable o : out real; v : real) is begin o := v; end procedure;
  procedure set_i (variable o : out integer; v : integer) is begin o := v; end procedure;
begin
  process
    variable r : rec := (0 ns, 0);
    variable u : rvec := (others => 0.0);
    variable i : integer := 2;
  begin
    set_t(r.t, 5 ns);
    set_r(u(i), 0.5);
    set_i(r.f, 9);
    report "t=" & time'image(r.t) & " f=" & integer'image(r.f);
    assert u(2) = 0.5 severity failure;
    set_t(r.t, 20 ns);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
