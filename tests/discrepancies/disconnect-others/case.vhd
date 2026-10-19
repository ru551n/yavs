-- `others` in a disconnection specification names the remaining guarded signals of the type (LRM 7.4).
entity top is end entity;
architecture a of top is
  function res (v : bit_vector) return bit is
  begin
    return v(v'low);
  end function;
  subtype rbit is res bit;
  signal b, c : rbit bus;
  disconnect b : rbit after 5 ns;
  disconnect others : rbit after 1 ns;
begin
end architecture;
