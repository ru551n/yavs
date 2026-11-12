-- Many `out` ports on elements of one array signal (`q(i)`): each is the only source of its
-- element and collapses into it. Deciding that looks at the sources of the shared leaf once,
-- not once per port (`elab_source_steps`).
entity cell is
  generic (IDX : natural);
  port (clk : in bit; q : out integer);
end;
architecture a of cell is
  signal r : integer := IDX;
begin
  q <= r;
  process (clk) begin
    if clk = '1' then r <= (r * 5 + IDX + 1) mod 1009; end if;
  end process;
end;

entity out_ports_shared is
  generic (N : natural := 600);
end;
architecture a of out_ports_shared is
  type iv is array (natural range <>) of integer;
  signal q : iv(0 to N - 1);
  signal clk : bit := '0';
begin
  g : for i in 0 to N - 1 generate
    u : entity work.cell generic map (i) port map (clk, q(i));
  end generate;
  process
    variable s : integer := 0;
  begin
    for k in 1 to 4 loop
      clk <= '1'; wait for 5 ns; clk <= '0'; wait for 5 ns;
    end loop;
    for i in q'range loop s := (s * 3 + q(i)) mod 1000003; end loop;
    report "sum=" & integer'image(s);
    wait;
  end process;
end;
