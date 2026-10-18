-- yavs-test: std=2008
-- expect: E1620 8:12 implicit signal attribute `delayed` of formal signal parameter `s` cannot be read in a subprogram
-- LRM 4.2.2.2
entity e is end entity;
architecture a of e is
  function f (signal s : bit) return bit is
  begin
    return s'delayed(1 ns);
  end function;
begin
end architecture;
