-- Reads of an out port in contexts one reference does not check: a case expression, a
-- report message, a constant initial value, a loop bound and a generic map actual.
entity leaf is generic (g : bit := '0'); port (o : out bit); end;
architecture a of leaf is begin o <= '1'; end;
entity top is port (p : out bit); end;
architecture a of top is
begin
  p <= '1';
  process begin
    case p is when '1' => null; when others => null; end case;
    report bit'image(p);
    for k in 0 to bit'pos(p) loop null; end loop;
    wait;
  end process;
  b : block constant c : bit := p; begin end block;
  u : entity work.leaf generic map (g => p) port map (o => open);
end;
