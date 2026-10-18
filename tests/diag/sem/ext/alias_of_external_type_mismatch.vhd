-- yavs-test: std=2008
-- expect: E1300 11:10 an alias of an external integer signal assigned a bit
-- LRM 6.6.2, 8.7
entity e is end entity;
architecture a of e is
  signal t : integer;
begin
  process
    alias s is <<signal .e.t : integer>>;
  begin
    s <= '1';
    wait;
  end process;
end architecture;
