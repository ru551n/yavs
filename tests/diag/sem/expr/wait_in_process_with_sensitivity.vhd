-- yavs-test: std=2008
-- expect: E1801 9:5 `wait` is not allowed in a process with a sensitivity list
-- LRM 10.2
entity e is end entity;
architecture a of e is
  signal s : bit;
begin
  process (s) begin
    wait for 1 ns;
  end process;
end architecture;
