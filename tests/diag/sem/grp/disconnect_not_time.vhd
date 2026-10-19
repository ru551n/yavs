-- yavs-test: std=2008
-- expect: E1300 12:29 not time
-- LRM 7.4
entity e is end entity;
architecture a of e is
  function res (v : bit_vector) return bit is
  begin
    return v(v'low);
  end function;
  subtype rbit is res bit;
  signal b : rbit bus;
  disconnect b : rbit after 5;
begin
end architecture;
