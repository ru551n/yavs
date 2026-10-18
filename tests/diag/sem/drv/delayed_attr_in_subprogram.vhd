-- rsim-test: std=2008
-- expect: E1620 9:12 implicit signal attribute `delayed` cannot be used in a subprogram
-- LRM 16.2
entity e is end entity;
architecture a of e is
  signal s : bit;
  function f return bit is
  begin
    return s'delayed(1 ns);
  end function;
begin
end architecture;
