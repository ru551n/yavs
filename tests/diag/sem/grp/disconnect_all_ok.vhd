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
  signal b, c : rbit bus;
  disconnect all : rbit after 1 ns;
begin
end architecture;
