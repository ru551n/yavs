entity top is end entity;
architecture a of top is
  type iv is array (natural range <>) of integer;
begin
  process
    variable x, y : integer;
    variable v : iv(0 to 2) := (1, 2, 3);
  begin
    (0 | 2 => x, 1 => y) := v;
    wait;
  end process;
end architecture;
