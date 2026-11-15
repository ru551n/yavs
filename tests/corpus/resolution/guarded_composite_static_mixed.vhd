-- difftest: top=guarded_composite_static_mixed std=2008 exit=1 ref=hand
-- ref=hand: a process drives elements 1 and 2 of a guarded array resolved as a whole by
-- static names and disconnects element 1 only (#1160). By the LRM its drivers are those of
-- elements 1 and 2 (14.7.2); at 1 ns one is disconnected and the other is not, which LRM
-- 14.7.3.2 makes an error: the run stops at 1 ns. Elements 0 and 3, which yavs gives the
-- process too (the source is the whole composite), take no part in that judgement
-- (decisions.md "Guarded record signals"). The expected text is yavs's wording. sim1 fails
-- the check at 0.5 ns and runs on past 1 ns; sim2 refuses the partial drivers of a resolved
-- composite.
entity guarded_composite_static_mixed is
end entity;

architecture sim of guarded_composite_static_mixed is
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
  signal g : rv bus := (others => 0);
begin
  p : process is
  begin
    g(1 to 2) <= (5, 6);
    g(1) <= transport null after 1 ns;
    wait for 500 ps;
    assert g = (0, 5, 6, 0);
    report "connected";
    wait for 1 ns;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
