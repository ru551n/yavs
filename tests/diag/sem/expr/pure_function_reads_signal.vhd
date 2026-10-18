-- yavs-test: std=2008
-- expect: E1830 9:12 pure function `f` reads signal `s` declared outside it
-- LRM 4.3
entity e is end entity;
architecture a of e is
  signal s : integer;
  function f return integer is
  begin
    return s;
  end function;
begin
end architecture;
