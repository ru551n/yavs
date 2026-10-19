-- LRM 6.5.6.3, 14.3.5: an open unconstrained in port with a default expression.
entity sub is port (p : in bit_vector := "101"); end entity;
architecture a of sub is
begin
  process begin report "RES length=" & integer'image(p'length); wait; end process;
end architecture;
entity top is end entity;
architecture a of top is
begin
  u : entity work.sub port map (p => open);
end architecture;
