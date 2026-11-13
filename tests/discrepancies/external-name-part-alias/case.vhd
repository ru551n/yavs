-- LRM 6.6.2, 8.7: an alias of a part (slice, element) of an external name
entity sub is end entity;
architecture r of sub is
  signal s : bit_vector(3 downto 0) := "0000";
begin
  s <= "1010" after 1 ns;
end architecture;
entity e is end entity;
architecture a of e is
begin
  u : entity work.sub;
  b : block
    alias sl is <<signal .e.u.s : bit_vector(3 downto 0)>>(3 downto 2);
    alias el is <<signal .e.u.s : bit_vector(3 downto 0)>>(1);
  begin
    process begin
      wait for 2 ns;
      report "RES " & to_string(sl) & " " & bit'image(el);
      wait;
    end process;
  end block;
end architecture;
