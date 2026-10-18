-- yavs-test: std=2008
-- expect: E1801 9:5 `wait` is not allowed in a process with a sensitivity list
-- LRM 11.3
entity e is end entity;
architecture a of e is
  signal s : bit;
begin
  process (all) begin
    wait for 1 ns;
  end process;
end architecture;
