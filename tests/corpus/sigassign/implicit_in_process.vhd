-- difftest: top=implicit_in_process std=2008 ignore=order
-- ignore=order: processes report in one cycle, ordered by yavs's rule (#252), which the
-- reference simulator does not follow here, so each process's reports are compared.
-- 'stable, 'quiet, 'delayed and 'transaction named inside process bodies (LRM 16.2.4: the
-- implicit signal belongs to the region enclosing the process), for scalars, a record and a
-- part of an array of records, in reads and in waits, in a loop and inside a for-generate (#240).
entity implicit_in_process is
end entity;

architecture test of implicit_in_process is
  type cell is record
    ok : boolean;
    n  : natural;
  end record;
  type cell_vec is array (natural range <>) of cell;
  signal i : integer := 0;
  signal b : bit := '0';
  signal r : cell := (true, 1);
  signal s : cell_vec(0 to 1) := ((false, 0), (true, 2));
begin
  drv : process
  begin
    wait for 1 ns;
    i <= 1;
    b <= '0';                           -- a transaction without an event
    wait for 0 ns;
    report "1ns+1 i'stable(2 ns) " & boolean'image(i'stable(2 ns))
      & " i'quiet " & boolean'image(i'quiet)
      & " i'delayed(1 ns) " & integer'image(i'delayed(1 ns));
    wait for 2 ns;
    report "3ns+1 i'stable(2 ns) " & boolean'image(i'stable(2 ns))
      & " i'quiet " & boolean'image(i'quiet)
      & " i'delayed(1 ns) " & integer'image(i'delayed(1 ns));
    r.n <= 4;
    s(1).ok <= true;                    -- active, no event
    wait for 3 ns;
    i <= 1;
    wait for 2 ns;
    i <= 2;
    wait for 1 ns;
    s(0).n <= 3;
    wait for 2 ns;
    r.n <= 5;
    wait;
  end process;

  mon : process
    variable n : natural := 0;
  begin
    wait for 5 ns;
    report "5ns i'stable " & boolean'image(i'stable)
      & " i'quiet(1 ns) " & boolean'image(i'quiet(1 ns))
      & " i'delayed(2 ns) " & integer'image(i'delayed(2 ns))
      & " b'quiet " & boolean'image(b'quiet)
      & " r'stable(2 ns) " & boolean'image(r'stable(2 ns))
      & " s(1)'quiet(1 ns) " & boolean'image(s(1)'quiet(1 ns))
      & " s(1)'stable " & boolean'image(s(1)'stable)
      & " s'delayed(1 ns)(1).ok " & boolean'image(s'delayed(1 ns)(1).ok);
    wait on i'transaction;
    n := n + 1;
    report "i'transaction " & integer'image(n) & " i'stable " & boolean'image(i'stable)
      & " i'quiet " & boolean'image(i'quiet);
    wait until not i'stable;
    report "i event, i'delayed(2 ns) " & integer'image(i'delayed(2 ns));
    wait until i'delayed(2 ns) = 2;
    report "i'delayed(2 ns) = 2, i'stable(1 ns) " & boolean'image(i'stable(1 ns));
    wait on r'transaction, s(0)'transaction;
    report "r'transaction, r'stable " & boolean'image(r'stable)
      & " r'delayed(1 ns).n " & integer'image(r'delayed(1 ns).n);
    wait;
  end process;

  -- The transaction of a bit signal toggles on every assignment, also without an event.
  tr : process
    variable last : bit;
  begin
    wait for 1 ns;
    last := b'transaction;
    wait for 0 ns;
    -- Named in a loop: the implicit signals still belong to the architecture.
    for k in 1 to 2 loop
      assert b'transaction /= last report "b'transaction did not toggle" severity failure;
      report "b'transaction toggled, b'quiet " & boolean'image(b'quiet)
        & " b'stable " & boolean'image(b'stable) & " " & integer'image(k);
    end loop;
    wait;
  end process;

  gen : for k in 0 to 1 generate
    p : process
    begin
      wait until not s(k)'quiet;
      report "s(" & integer'image(k) & ") active, stable " & boolean'image(s(k)'stable);
      wait;
    end process;
  end generate;
end architecture;
