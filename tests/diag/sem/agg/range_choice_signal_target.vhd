-- yavs-test: std=2008
-- expect: E1333 11:4 range choice in a signal aggregate target
-- expect: E1333 11:22 second range choice of the same target
-- LRM 10.5.2.1 (user decision, 2026-10-19)
-- Same for the signal assignment form.
entity e is end entity;
architecture a of e is
  signal sa, sb : bit_vector(2 downto 0);
  signal v : bit_vector(5 downto 0);
begin
  (5 downto 3 => sa, 2 downto 0 => sb) <= v;
end architecture;
