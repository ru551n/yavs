-- yavs-test: std=2008
-- expect: E1335 16:12 v(1) twice
-- LRM 10.6.2.1
entity e is end entity;
architecture a of e is
  type iv is array (natural range <>) of integer;
  type rec is record a, b : integer; end record;
  signal s, t : integer;
  signal sv : iv(0 to 2);
begin
  process
    variable x, y : integer;
    variable v : iv(0 to 2);
    variable r : rec;
  begin
    (v(1), v(1)) := iv'(1, 2);
    wait;
  end process;
end architecture;
