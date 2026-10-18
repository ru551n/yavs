-- yavs-test: std=2008
-- expect: E1405 10:10 length mismatch: target has 3 elements, value has 5
-- LRM 10.6.2
entity e is end entity;
architecture a of e is
begin
  process
    variable v : string(1 to 3);
  begin
    v := "hello";
    wait;
  end process;
end architecture;
