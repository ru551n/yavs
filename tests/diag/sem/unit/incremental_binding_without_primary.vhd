-- yavs-test: std=2008
-- expect: E1704 16:18 this binding adds maps, but no configuration specification binds the instance to an entity
-- LRM 7.3.2.1 (incremental binding needs a primary binding; GHDL, Questa reject, NVC does not support it)
entity inv is generic (W : natural := 1); end;
architecture rtl of inv is signal s : bit_vector(W - 1 downto 0); begin end;
entity top is end;
architecture t of top is
  component inv is generic (W : natural := 1); end component;
  for u2 : inv use entity work.inv(rtl);
begin
  u1 : inv;
  u2 : inv;
end;
configuration cfg of top is
  for t
    for u1 : inv generic map (W => 3); end for;
  end for;
end;
