-- A signal name with a subscript that is not static as the actual of an `in` port.
entity sub is
  port (d : in bit);
end entity;
architecture a of sub is
begin
  process (d) begin report "d=" & bit'image(d); end process;
end architecture;

entity top is end entity;
architecture a of top is
  signal q : bit_vector(0 to 1) := "01";
  signal r : natural range 0 to 1 := 0;
begin
  u : entity work.sub port map (d => q(r));
  process begin wait for 1 ns; r <= 1; wait; end process;
end architecture;
