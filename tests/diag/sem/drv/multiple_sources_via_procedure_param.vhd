-- yavs-test: std=2008
-- expect: E1812 16:28 unresolved signal `s` has multiple sources
-- LRM 14.7.2
-- A signal parameter that the procedure assigns is a source of the actual in every process
-- that calls the procedure (#42).
entity multiple_sources_via_procedure_param is
end entity;
architecture a of multiple_sources_via_procedure_param is
  signal s : bit;
  procedure drive(signal o : out bit) is
  begin
    o <= '1';
  end procedure;
begin
  p1 : process begin drive(s); wait; end process;
  p2 : process begin drive(s); wait; end process;
end architecture;
