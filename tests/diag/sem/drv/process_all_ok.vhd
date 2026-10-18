-- yavs-test: std=2008
-- expect: none
-- LRM 11.3
entity e is end entity;
architecture a of e is
  signal s, t : bit;
begin
  process (all) begin
    t <= s;
  end process;
end architecture;
