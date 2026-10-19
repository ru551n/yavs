-- difftest: top=record_port_parts_nested std=2008 ref=hand
-- Individual association of a record port with unconstrained elements (LRM 6.5.7.1, #288)
-- below a nested record: each element takes its bounds from its actual, an element named index
-- by index runs from its lowest to its highest index. ref=hand: the reference simulator fails
-- elaboration ("formal length 0"); .expected is the output of the other two reference
-- simulators, which agree.
package rpn_p is
  type in_t is record
    a : bit;
    d : bit_vector;
    n : integer_vector;
  end record;
  type nest_t is record
    i : in_t;
    k : bit_vector;
  end record;
  type out_t is record
    q : bit_vector;
    v : bit;
  end record;
end package;

use work.rpn_p.all;
entity slave is
  port (pin : in nest_t; pout : out out_t);
end entity;
architecture a of slave is
begin
  process
  begin
    report "d " & integer'image(pin.i.d'left) & " " & integer'image(pin.i.d'right)
      & " n " & integer'image(pin.i.n'left) & " " & integer'image(pin.i.n'right)
      & " k " & integer'image(pin.k'left) & " " & integer'image(pin.k'right)
      & " q " & integer'image(pout.q'length);
    pout.q <= (pout.q'range => '1');
    pout.v <= '1';
    wait for 1 ns;
    report "a=" & bit'image(pin.i.a) & " d=" & to_string(pin.i.d) & " n0=" & integer'image(pin.i.n(0))
      & " k=" & to_string(pin.k);
    wait;
  end process;
end architecture;

use work.rpn_p.all;
entity record_port_parts_nested is end entity;
architecture a of record_port_parts_nested is
  signal sa : bit;
  signal sd : bit_vector(7 downto 0);
  signal sw : bit_vector(0 to 15) := x"00f0";
  signal n0, n1 : integer;
  signal sq : bit_vector(2 to 4);
  signal sv : bit;
begin
  i : entity work.slave port map (
    pin.i.a => sa,
    pin.i.d => sd,
    pin.i.n(0) => n0,
    pin.i.n(1) => n1,
    pin.k => sw(4 to 9),
    pout.q => sq,
    pout.v => sv);
  process
  begin
    sa <= '1';
    sd <= x"a5";
    n0 <= 7;
    wait for 2 ns;
    report "sq=" & to_string(sq) & " sv=" & bit'image(sv);
    assert sq = "111" and sv = '1' severity failure;
    wait;
  end process;
end architecture;
