-- yavs-test: std=2008
-- expect: none
-- A 2-D aggregate with named choices: the inner aggregate's choices are checked against the
-- second dimension's index type, not the first's (#621, HIR validation in debug builds).
entity func_test3 is end entity;
architecture fum of func_test3 is
  type remains is (r0, r1);
  type branch is array (remains, bit) of remains;
  constant br_table : branch := (r0 => ('0' => r0, '1' => r1), r1 => ('0' => r1, '1' => r0));
begin
end architecture;
