-- yavs-test: std=2008
-- expect: E1801 22:7 procedure `outer` contains a `wait` and is called by a process with a sensitivity list
-- expect: E1801 24:5 procedure `pause` contains a `wait` and is called by a process with a sensitivity list
-- expect: E1801 26:23 procedure `outer` contains a `wait` and is called by a process with a sensitivity list
-- LRM 10.2, 11.3: directly, through another procedure, and for `process (all)`
entity e is end entity;
architecture a of e is
  signal s : bit;
  procedure pause is
  begin
    wait for 1 ns;
  end procedure;
  procedure outer (n : integer) is
  begin
    if n > 0 then outer(n - 1); else pause; end if;
  end procedure;
  procedure fine is begin null; end procedure;
begin
  process (s) begin
    fine;
    if s = '1' then
      outer(3);
    end if;
    pause;
  end process;
  process (all) begin outer(1); end process;
  process begin pause; wait; end process;
  pause;
end architecture;
