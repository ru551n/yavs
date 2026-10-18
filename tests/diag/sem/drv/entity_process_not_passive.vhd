-- yavs-test: std=2008
-- expect: E1813 7:20 a statement in an entity statement part cannot drive a signal
-- LRM 11.1
entity e is
  port (o : out bit);
begin
  p: process begin o <= '1'; wait; end process;
end entity;
