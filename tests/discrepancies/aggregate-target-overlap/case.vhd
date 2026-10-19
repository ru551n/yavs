entity top is end entity;
architecture a of top is
  type iv is array (natural range <>) of integer;
begin
  process
    variable v : iv(0 to 2) := (1, 2, 3);
  begin
    (v(0 to 1), v(1 to 2)) := iv'(1, 2, 3, 4);
    wait;
  end process;
end architecture;
