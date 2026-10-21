-- difftest: top=incremental_binding_cfg std=2008 ref=hand
-- ref=hand: sim1 does not support incremental binding ("binding indication with generic map
-- aspect and OPEN entity aspect"); sim3 accepts it. LRM 7.3.2.3: u2's configuration adds
-- W => 3 to the primary binding of its configuration specification; u1 keeps the default.
entity inv is generic (W : natural := 1); end;
architecture rtl of inv is
begin
  process begin report "W=" & integer'image(W); wait; end process;
end;
entity incremental_binding is end;
architecture t of incremental_binding is
  component inv is generic (W : natural := 1); end component;
  for u2 : inv use entity work.inv(rtl);
begin
  u1 : inv;
  u2 : inv;
end;
configuration incremental_binding_cfg of incremental_binding is
  for t
    for u2 : inv generic map (W => 3); end for;
  end for;
end;
