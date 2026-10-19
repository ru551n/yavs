-- difftest: top=record_port_parts_open std=2008
-- A record port with unconstrained elements associated individually takes each element's
-- bounds from its actual (LRM 6.5.7.1, #288): an in port's elements from signal names (a whole
-- signal and a slice), an out port's from the signals it drives.
package rpo_p is
  type in_t is record
    a : bit;
    d : bit_vector;
    k : bit_vector;
  end record;
  type out_t is record
    q : bit_vector;
    v : bit;
  end record;
end package;

use work.rpo_p.all;
entity slave is
  port (pin : in in_t; pout : out out_t);
end entity;
architecture a of slave is
begin
  process
  begin
    report "d " & integer'image(pin.d'left) & " " & integer'image(pin.d'right)
      & " k " & integer'image(pin.k'left) & " " & integer'image(pin.k'right)
      & " q " & integer'image(pout.q'left) & " " & integer'image(pout.q'right);
    pout.q <= (pout.q'range => '1');
    pout.v <= '1';
    wait for 1 ns;
    report "a=" & bit'image(pin.a) & " d=" & to_string(pin.d) & " k=" & to_string(pin.k);
    wait;
  end process;
end architecture;

use work.rpo_p.all;
entity record_port_parts_open is end entity;
architecture a of record_port_parts_open is
  signal sa : bit;
  signal sd : bit_vector(7 downto 0);
  signal sw : bit_vector(0 to 15) := x"00f0";
  signal sq : bit_vector(2 to 4);
  signal sv : bit;
begin
  i : entity work.slave port map (
    pin.a => sa,
    pin.d => sd,
    pin.k => sw(4 to 9),
    pout.q => sq,
    pout.v => sv);
  process
  begin
    sa <= '1';
    sd <= x"a5";
    wait for 2 ns;
    report "sq=" & to_string(sq) & " sv=" & bit'image(sv);
    assert sq = "111" and sv = '1' severity failure;
    wait;
  end process;
end architecture;
