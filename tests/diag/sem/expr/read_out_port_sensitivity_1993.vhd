-- yavs-test: std=1993
-- expect: E1360 9:12 cannot read port `o` of mode `out`
-- LRM 1993 4.3.2, 8.1
entity e is
  port (o : out bit);
end entity;
architecture a of e is
begin
  process (o) begin
    null;
  end process;
end architecture;
