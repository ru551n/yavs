-- difftest: top=guarded_composite_partly_null std=2008 exit=1 ref=hand
-- ref=hand: a slice of a guarded array resolved as a whole, with a bound that is not static,
-- driven with a null waveform element (#1160). The longest static prefix of the target is the
-- whole signal, so the process has a driver for every element (LRM 14.7.2); at 1 ns elements 1
-- and 2 are disconnected and 0 and 3 are not, and LRM 14.7.3.2 makes such a driving value an
-- error: the run stops at 1 ns. The expected text is yavs's wording. sim1 fails the check at
-- 0.5 ns and runs on past 1 ns; sim2 stops with an internal error.
entity guarded_composite_partly_null is
end entity;

architecture sim of guarded_composite_partly_null is
  type rec is record
    a : integer;
    b : bit_vector(0 to 1);
  end record;
  type ra is array (natural range <>) of rec;
  type ras is array (natural range <>) of ra(0 to 3);
  function resa(x : ras) return ra is
  begin
    if x'length = 0 then
      return (0 to 3 => (-1, "11"));
    end if;
    return x(x'left);
  end function;
  subtype rra is resa ra(0 to 3);
  signal gr : rra bus := (others => (0, "00"));
  signal k : natural := 1;
begin
  p : process is
  begin
    gr(k to 2) <= (others => (3, "01")), null after 1 ns;
    wait for 500 ps;
    assert gr(1).a = 3 and gr(0).a = 0;
    report "connected";
    wait for 1 ns;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
