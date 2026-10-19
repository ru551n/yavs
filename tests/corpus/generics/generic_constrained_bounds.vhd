-- difftest: top=generic_constrained_bounds std=2008
-- An array generic of a constrained subtype takes the bounds of its subtype, not those of
-- the actual (implicit subtype conversion): slices and indices use the generic's bounds.
entity gcb_sub is
  generic (g : bit_vector(3 downto 0); h : string(1 to 3); k : bit_vector);
end entity;
architecture a of gcb_sub is
  signal s : bit_vector(1 downto 0) := g(3 downto 2);
begin
  process
  begin
    report "g " & integer'image(g'left) & " downto " & integer'image(g'right)
      & " = " & to_string(g) & ", g(1 downto 0) = " & to_string(g(1 downto 0));
    report "h " & integer'image(h'left) & " to " & integer'image(h'right) & " = " & h
      & ", h(3) = " & h(3);
    report "k " & integer'image(k'left) & " to " & integer'image(k'right);
    report "s = " & to_string(s);
    wait;
  end process;
end architecture;

entity generic_constrained_bounds is end entity;
architecture a of generic_constrained_bounds is
  constant c : string(5 to 7) := "xyz";
begin
  u : entity work.gcb_sub generic map (g => "1100", h => c, k => "01");
end architecture;
