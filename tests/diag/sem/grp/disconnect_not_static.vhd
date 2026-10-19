-- yavs-test: std=2008
-- expect: E1410 13:29 not static
-- LRM 7.4
entity e is end entity;
architecture a of e is
  function res (v : bit_vector) return bit is
  begin
    return v(v'low);
  end function;
  subtype rbit is res bit;
  signal b : rbit bus;
  signal n : integer;
  disconnect b : rbit after n * 1 ns;
begin
end architecture;
