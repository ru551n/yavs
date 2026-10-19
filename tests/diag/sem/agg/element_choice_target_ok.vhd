-- yavs-test: std=2008
-- expect: none
-- LRM 10.5.2.1 (user decision, 2026-10-19)
-- Positional and element-choice aggregate targets stay legal, and so do range choices in
-- aggregate values and in slice targets.
entity e is end entity;
architecture a of e is
  signal x, y : bit;
  signal v : bit_vector(1 downto 0);
begin
  process
    variable p, q : bit;
    variable w : bit_vector(3 downto 0);
  begin
    (p, q) := v;
    (1 => p, 0 => q) := v;
    (x, y) <= v;
    (1 => x, 0 => y) <= v;
    w(3 downto 2) := (1 downto 0 => p);
    wait;
  end process;
end architecture;
