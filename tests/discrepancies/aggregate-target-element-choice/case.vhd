-- LRM 10.6.2.1, 9.3.3.3: element choices of an aggregate target mapped to a value whose bounds differ from the choices.
entity top is end entity;
architecture a of top is
  type iv is array (natural range <>) of integer;
begin
  process
    variable v : iv(1 downto 0) := (10, 0);
    variable a, b : integer;
  begin
    (0 => a, 1 => b) := v;
    report "RES a=" & integer'image(a) & " b=" & integer'image(b);
    wait;
  end process;
end architecture;
