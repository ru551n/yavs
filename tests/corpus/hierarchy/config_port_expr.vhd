-- difftest: top=config_port_expr_cfg std=2008 stop=10ns
-- Expression actuals in the port map of a binding indication (LRM 6.5.6.3, 7.3.2, #74): the
-- equivalent process `formal <= expr` reads the component's ports, in a configuration
-- specification and in a configuration declaration.
entity config_port_expr_leaf is
  generic (k : integer := 0);
  port (n : in integer; m : in integer := 0);
end;
architecture a of config_port_expr_leaf is
begin
  process (n, m) begin
    report "k=" & integer'image(k) & " n=" & integer'image(n) & " m=" & integer'image(m) & " @" & time'image(now);
  end process;
end;
entity config_port_expr is end;
architecture a of config_port_expr is
  component comp is
    generic (k : integer := 1);
    port (c : in integer; d : in integer);
  end component;
  signal x : integer := 1;
  for u2 : comp use entity work.config_port_expr_leaf port map (n => c + d, m => k * c);
begin
  u1 : comp port map (c => x, d => 10);
  u2 : comp generic map (k => 2) port map (c => x, d => x + 1);
  process begin
    wait for 1 ns;
    x <= 4;
    wait;
  end process;
end;
configuration config_port_expr_cfg of config_port_expr is
  for a
    for u1 : comp
      use entity work.config_port_expr_leaf port map (n => c * 3, m => d);
    end for;
  end for;
end;
