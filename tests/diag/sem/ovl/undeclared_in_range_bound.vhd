-- yavs-test: std=2008
-- expect: E1100 22:19 m1 is not declared
-- expect: E1100 23:19 m2 is not declared
-- expect: E1100 24:14 m3 is not declared
-- expect: E1100 25:14 m4 is not declared
-- expect: E1100 25:24 m5 is not declared
-- expect: E1100 26:23 m6 is not declared
-- expect: E1100 27:19 count_x is not declared
-- expect: E1100 28:31 m7 is not declared
-- expect: E1100 31:21 m8 is not declared
-- help: similar names exist
-- LRM 12.3, 5.2.1 (a range bound with no visible declaration is the error, not an undetermined range type)
entity e3 is end entity;
architecture a of e3 is
  constant count : integer := 3;
  type arr is array (0 to 2) of integer;
  signal sg : bit_vector(0 to 3);
begin
  process
    variable i : integer;
  begin
    for k in 0 to m1 loop end loop;
    for k in 0 to m2(1) loop end loop;
    for k in m3 to 3 loop end loop;
    for k in m4 downto m5 loop end loop;
    for k in 0 to 1 + m6 loop end loop;
    for k in 0 to count_x loop end loop;
    for k in 0 to 2 loop i := m7 + k; end loop;
    wait;
  end process;
  g : for j in 0 to m8 generate end generate;
end architecture;
