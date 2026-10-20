-- yavs-test: std=2008
-- expect: E1812 14:22 unresolved signal `t` has multiple sources
-- LRM 14.7.2
-- A procedure declared in a process that assigns an enclosing signal makes the process
-- a source of it (#42).
entity multiple_sources_via_process_procedure is
end entity;
architecture a of multiple_sources_via_process_procedure is
  signal t : bit;
begin
  p3 : process
    procedure drive2 is begin t <= '1'; end procedure;
  begin drive2; wait; end process;
  p4 : process begin t <= '0'; wait; end process;
end architecture;
