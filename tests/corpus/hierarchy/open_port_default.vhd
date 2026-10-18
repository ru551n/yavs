-- difftest: top=open_port_default std=2008 stop=1ns
-- An unconstrained in port left open or unassociated takes the bounds of its default (#218).
entity sub is
  port (p : in bit_vector := "101"; q : in bit_vector := "1100");
end entity;
architecture a of sub is
begin
  process
  begin
    report "p'length = " & integer'image(p'length) & " q'length = " & integer'image(q'length);
    report "p'left = " & integer'image(p'left) & " q'right = " & integer'image(q'right);
    assert p = "101" and q = "1100" severity failure;
    wait;
  end process;
end architecture;
entity open_port_default is
end entity;
architecture a of open_port_default is
begin
  u1 : entity work.sub port map (p => open, q => open);
  u2 : entity work.sub;
end architecture;
