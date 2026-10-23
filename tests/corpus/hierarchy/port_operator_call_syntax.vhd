-- difftest: top=port_operator_call_syntax std=2008 ref=hand
-- A predefined operator in call syntax as an actual, "not"(a), is an expression actual, not a
-- conversion function (a conversion needs a function with a body). Expected output by hand,
-- checked against the second reference simulator; the first stops in elaboration with
-- "missing body for STD.STANDARD."not"".
entity and1 is
  port (s, t : in bit; u : out bit);
end entity;

architecture a of and1 is
begin
  u <= s and t;
end architecture;

entity port_operator_call_syntax is end entity;

architecture a of port_operator_call_syntax is
  component and1 is
    port (s, t : in bit; u : out bit);
  end component;
  signal a, d, e, f : bit;
begin
  x1 : and1 port map (s => "not"(a), t => "not"(d), u => e);
  x2 : and1 port map (s => not a, t => not d, u => f);
  process
  begin
    wait for 1 ns;
    report bit'image(e) & bit'image(f);
    a <= '1';
    wait for 1 ns;
    report bit'image(e) & bit'image(f);
    d <= '1';
    wait for 1 ns;
    report bit'image(e) & bit'image(f);
    wait;
  end process;
end architecture;
