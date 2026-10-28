-- yavs-test: std=2008
-- expect: E1360 16:7 the actual of signal parameter `x` must be a static name
-- LRM 4.2.2.3: the actual of a signal parameter is a static signal name (#813)
entity e is
  generic (g : integer := 2);
end entity;
architecture a of e is
  procedure p (signal x : bit) is
  begin
  end procedure;
  signal b : bit_vector(1 to 4);
begin
  process
    variable i : integer := 1;
  begin
    p(b(i));
    p(b(g));
    p(b(1));
    wait;
  end process;
end architecture;
