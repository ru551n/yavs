-- yavs-test: std=2008
-- expect: E1612 9:15 an alias of an external name has no subtype indication
-- LRM 6.6.2
entity e is end entity;
architecture a of e is
  signal t : bit;
begin
  process
    alias s : bit is <<signal .e.t : bit>>;
  begin
    wait;
  end process;
end architecture;
