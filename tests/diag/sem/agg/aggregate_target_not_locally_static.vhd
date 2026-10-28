-- yavs-test: std=2008
-- expect: E1410 13:6 an element of an aggregate target must be a locally static name
-- expect: E1410 15:4 an element of an aggregate target must be a locally static name
-- expect: E1410 21:6 an element of an aggregate target must be a locally static name
-- LRM 10.5.2.1, 10.6.2.1: each element of an aggregate target is a locally static name (#812)
entity e is
  generic (n : integer := 1);
end entity;
architecture a of e is
  signal s, t : bit_vector(0 to 3);
begin
  g : for i in 0 to 1 generate
    (s(i), t(i)) <= bit_vector'("10");
  end generate;
  (s(n), t(n)) <= bit_vector'("10");
  (s(3), t(3)) <= bit_vector'("10");
  process
    variable v, w : bit_vector(0 to 3);
    variable k : integer := 1;
  begin
    (v(k), w(k)) := bit_vector'("10");
    (v(0), w(0 to 1)) := bit_vector'("101");
    wait;
  end process;
end architecture;
