-- yavs-test: std=2008
-- expect: E1360 12:16 a subprogram parameter cannot be declared `bus`
-- LRM 4.2.2.3: `bus` does not appear in a formal signal parameter declaration (#808)
entity e is
end entity;
architecture a of e is
  function res (v : bit_vector) return bit is
  begin
    return v(v'left);
  end function;
  subtype rbit is res bit;
  procedure p (signal q : out rbit bus) is
  begin
    q <= '1';
  end procedure;
begin
end architecture;
