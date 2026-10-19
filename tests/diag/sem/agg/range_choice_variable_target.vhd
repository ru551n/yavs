-- yavs-test: std=2008
-- expect: E1333 18:6 range choice in a variable aggregate target
-- expect: E1333 18:24 second range choice of the same target
-- expect: E1333 19:12 range choice in a nested aggregate target
-- expect: E1333 19:30 second range choice of the nested target
-- LRM 10.5.2.1 (user decision, 2026-10-19)
-- A range choice in an aggregate target is rejected: the LRM does not say how it maps the value.
entity e is end entity;
architecture a of e is
  type t is array (0 to 1) of bit_vector(3 downto 0);
begin
  process
    variable v : bit_vector(5 downto 0);
    variable u : t;
    variable hi, w : bit_vector(3 downto 0);
    variable lo : bit_vector(1 downto 0);
  begin
    (5 downto 2 => hi, 1 downto 0 => lo) := v;
    (0 => (3 downto 2 => lo, 1 downto 0 => lo), 1 => w) := u;
    wait;
  end process;
end architecture;
