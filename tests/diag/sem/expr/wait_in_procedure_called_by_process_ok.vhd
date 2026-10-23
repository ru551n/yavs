-- yavs-test: std=2008
-- expect: none
-- LRM 11.3: a process without a sensitivity list, a concurrent call and a procedure that waits
-- but is never called by a process with a sensitivity list
entity e is end entity;
architecture a of e is
  signal s : bit;
  procedure pause is
  begin
    wait for 1 ns;
  end procedure;
  procedure loops (n : integer) is
  begin
    if n > 0 then loops(n - 1); end if;
  end procedure;
begin
  process (s) begin loops(2); end process;
  process begin pause; wait; end process;
  pause;
end architecture;
