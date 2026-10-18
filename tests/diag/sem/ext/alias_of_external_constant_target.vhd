-- yavs-test: std=2008
-- expect: E1601 11:5 an alias of an external constant is not an assignment target
-- LRM 6.6.2, 8.7
entity e is end entity;
architecture a of e is
  constant c : integer := 1;
begin
  process
    alias k is <<constant .e.c : integer>>;
  begin
    k <= 2;
    wait;
  end process;
end architecture;
