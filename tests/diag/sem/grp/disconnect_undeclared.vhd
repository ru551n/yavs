-- yavs-test: std=2008
-- expect: E1100 11:14 undeclared
-- LRM 7.4
entity e is end entity;
architecture a of e is
  function res (v : bit_vector) return bit is
  begin
    return v(v'low);
  end function;
  subtype rbit is res bit;
  disconnect nothing : rbit after 5 ns;
begin
end architecture;
