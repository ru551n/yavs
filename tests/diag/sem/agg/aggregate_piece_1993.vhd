-- yavs-test: std=1993
-- expect: E1007 9:4 an element of type `BIT` is expected: an array value as a part of an aggregate is not allowed in VHDL-1993
-- LRM 1993 7.3.2.2: each element association gives one element; array pieces are VHDL-2008 (#814)
entity e is end entity;
architecture a of e is
  signal s, t : bit_vector(0 to 7);
begin
  (s(0), s(1), s(2), s(3), s(4), s(5), s(6), s(7)) <= bit_vector'("10101010");
  (t(0 to 5), t(6), t(7)) <= bit_vector'("10101010");
end architecture;
