-- A guarded signal has at most one disconnection specification (LRM 7.4).
entity top is end entity;
architecture a of top is
  function res (v : bit_vector) return bit is
  begin
    return v(v'low);
  end function;
  subtype rbit is res bit;
  signal b : rbit bus;
  disconnect b : rbit after 5 ns;
  disconnect b : rbit after 6 ns;
begin
end architecture;
