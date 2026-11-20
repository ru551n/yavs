-- yavs-test: std=2019
-- expect: E1100 9:29 an undeclared name as a range is one error, also with range records in use
-- LRM 12.3: every name used must be declared
entity e is end entity;
architecture a of e is
  subtype ir is integer'range'record;
begin
  process
    variable v : bit_vector(xyz);
  begin
    wait;
  end process;
end architecture;
