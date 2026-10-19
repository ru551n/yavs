-- yavs-test: std=2008
-- expect: none
-- LRM 7.4
entity e is end entity;
architecture a of e is
  function res (v : bit_vector) return bit is
  begin
    return v(v'low);
  end function;
  subtype rbit is res bit;
  signal b : rbit bus;
  signal r : rbit register;
  signal c, d : rbit bus;
  signal e : rbit bus;
  disconnect b : rbit after 5 ns;
  disconnect r, c : rbit after 2 * 3 ns;
  disconnect others : rbit after 1 ns;
begin
end architecture;
