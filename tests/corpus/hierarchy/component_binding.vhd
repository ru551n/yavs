-- difftest: top=component_binding std=2008
entity and2 is port (a, b : in bit; y : out bit); end;
architecture a of and2 is begin y <= a and b; end;
entity component_binding is end;
architecture t of component_binding is
  component and2 is port (a, b : in bit; y : out bit); end component;
  signal a, b, y : bit := '0';
begin
  u : and2 port map (a => a, b => b, y => y);
  process begin
    wait for 1 ns; report "y=" & bit'image(y);
    a <= '1'; wait for 1 ns; report "y=" & bit'image(y);
    b <= '1'; wait for 1 ns; report "y=" & bit'image(y);
    wait;
  end process;
end;
