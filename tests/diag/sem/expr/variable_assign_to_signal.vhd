-- yavs-test: std=2008
-- expect: E1360 9:5 `s` is a signal; use `<=` to assign it
-- LRM 10.6
entity e is end entity;
architecture a of e is
  signal s : integer;
begin
  process begin
    s := 1;
    wait;
  end process;
end architecture;
