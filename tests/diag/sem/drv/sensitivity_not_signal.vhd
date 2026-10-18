-- yavs-test: std=2008
-- expect: E1360 8:12 `k` in a sensitivity list must be a signal
-- LRM 11.3
entity e is end entity;
architecture a of e is
  constant k : integer := 1;
begin
  process (k) begin
    null;
  end process;
end architecture;
