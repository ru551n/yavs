-- yavs-test: std=2008
-- expect: E1100 11:14 `cnt` is not declared
-- help: a signal with a similar name exists: `count`
-- LRM 12.3
entity e is end entity;
architecture a of e is
  signal count : integer;
begin
  process
  begin
    count <= cnt + 1;
    wait;
  end process;
end architecture;
