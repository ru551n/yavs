-- difftest: top=aggregate_target_matrix std=2008 ref=hand
-- Aggregate targets of multi-dimensional arrays (#350, LRM 10.5.2.1, 10.6.2.1, 9.3.3.3; lrm-checks
-- L98): an aggregate of aggregates, one per row, the scalar elements named in the last one;
-- integer, real and std_logic elements, three dimensions, index choices, variables and signals.
-- ref=hand: the first reference simulator gives every row the first row's elements, so the
-- .expected is the LRM result.
library ieee;
use ieee.std_logic_1164.all;
entity aggregate_target_matrix is end entity;
architecture t of aggregate_target_matrix is
  type m2 is array (0 to 1, 0 to 1) of integer;
  type mr is array (1 to 2, 1 to 2) of real;
  type ml is array (0 to 1, 0 to 1) of std_logic;
  type m3 is array (0 to 1, 0 to 1, 0 to 1) of integer;
  type md is array (3 downto 2, 0 to 1) of integer;
  function mk(b : integer) return m2 is
  begin
    return ((b, b + 1), (b + 2, b + 3));
  end function;
  signal s0, s1, s2, s3 : integer := 0;
  signal l0, l1, l2, l3 : std_logic := 'U';
begin
  process
    variable p0, p1, p2, p3, p4, p5, p6, p7 : integer;
    variable q0, q1, q2, q3 : std_logic;
    variable x0, x1, x2, x3 : real;
  begin
    ((p0, p1), (p2, p3)) := mk(10);
    assert p0 = 10 and p1 = 11 and p2 = 12 and p3 = 13 severity failure;
    ((p0, p1), (p2, p3)) := m2'((20, 21), (22, 23));
    assert p0 = 20 and p1 = 21 and p2 = 22 and p3 = 23 severity failure;
    (1 => (1 => p0, 0 => p1), 0 => (1 => p2, 0 => p3)) := mk(30);
    assert p3 = 30 and p2 = 31 and p1 = 32 and p0 = 33 severity failure;
    ((x0, x1), (x2, x3)) := mr'((1.5, 2.5), (3.5, 4.5));
    assert x0 = 1.5 and x1 = 2.5 and x2 = 3.5 and x3 = 4.5 severity failure;
    ((q0, q1), (q2, q3)) := ml'(('1', '0'), ('Z', 'X'));
    assert q0 = '1' and q1 = '0' and q2 = 'Z' and q3 = 'X' severity failure;
    (((p0, p1), (p2, p3)), ((p4, p5), (p6, p7))) := m3'(((1, 2), (3, 4)), ((5, 6), (7, 8)));
    assert p0 = 1 and p1 = 2 and p2 = 3 and p3 = 4 and p4 = 5 and p5 = 6 and p6 = 7 and p7 = 8 severity failure;
    ((p0, p1), (p2, p3)) := md'((10, 11), (12, 13));
    assert p0 = 10 and p1 = 11 and p2 = 12 and p3 = 13 severity failure;
    ((s0, s1), (s2, s3)) <= mk(40);
    ((l0, l1), (l2, l3)) <= ml'(('1', '0'), ('Z', 'X')) after 2 ns;
    wait for 1 ns;
    assert s0 = 40 and s1 = 41 and s2 = 42 and s3 = 43 severity failure;
    assert l0 = 'U' and l3 = 'U' severity failure;
    wait for 1 ns;
    assert l0 = '1' and l1 = '0' and l2 = 'Z' and l3 = 'X' severity failure;
    (1 => (1 => s0, 0 => s1), 0 => (1 => s2, 0 => s3)) <= mk(50);
    wait for 1 ns;
    assert s3 = 50 and s2 = 51 and s1 = 52 and s0 = 53 severity failure;
    report "p0=" & integer'image(p0) & " s0=" & integer'image(s0) & " l3=" & std_logic'image(l3);
    wait;
  end process;
end architecture;
