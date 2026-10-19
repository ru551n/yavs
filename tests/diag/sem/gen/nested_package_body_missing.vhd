-- yavs-test: std=2008
-- expect: E1230 7:14 function `f` of a nested package has no body
-- LRM 4.7, 4.8
entity e is end;
architecture a of e is
  package p is
    function f return integer;
  end package;
begin
end;
