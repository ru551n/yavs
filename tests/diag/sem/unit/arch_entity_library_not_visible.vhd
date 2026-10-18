-- yavs-test: std=2008
-- expect: E1100 4:19 `ieee` is not declared
-- LRM 3.3.1, 13.2 (only `std` and `work` are implicitly visible library names)
architecture a of ieee.std_logic_1164 is
begin
end architecture;
