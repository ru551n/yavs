-- yavs-test: std=2008
-- expect: E1230 6:12 function `first` has no body
-- LRM 4.2.1, 4.3: a header-instance spec in an architecture without its body (#1136)
entity e is end;
architecture a of e is
  function first generic (type t) generic map (t => bit) parameter (v : t) return integer;
begin
end;
