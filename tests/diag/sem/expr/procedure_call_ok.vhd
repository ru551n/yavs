-- yavs-test: std=2008
-- expect: none
-- LRM 10.7 (procedure call statement)
entity e is end entity;
architecture x of e is
  procedure a (constant c : in integer) is
  begin
  end procedure;
begin
  process begin
    a(1);
    wait;
  end process;
end architecture;
