-- yavs-test: std=2008
-- expect: E1100 7:32 no generic `u`
-- LRM 4.2.1, 6.5.7.2: the header generic map associates the header's own generics
entity e is end;
architecture a of e is
  function f generic (type t; constant n : integer := 1)
    generic map (t => integer, u => 2) parameter (v : t) return t is
  begin
    return v;
  end function;
begin
end;
