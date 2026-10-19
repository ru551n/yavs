-- yavs-test: std=2008
-- expect: E1650 12:14 other type
-- LRM 7.4
entity e is end entity;
architecture a of e is
  function res (v : bit_vector) return bit is
  begin
    return v(v'low);
  end function;
  subtype rbit is res bit;
  signal b : rbit bus;
  disconnect b : integer after 5 ns;
begin
end architecture;
