-- yavs-test: std=2008
-- expect: E1237 11:13 an instance in a subprogram made before the body, which declares a shared variable
-- LRM 4.9, 4.3
entity e is end;
architecture a of e is
  package g is
    generic (n : integer);
    function f return integer;
  end package;
  function h return integer is
    package i is new g generic map (n => 1);
  begin
    return 1;
  end function;
  package body g is
    type pt is protected
    end protected;
    type pt is protected body
    end protected body;
    shared variable v : pt;
    function f return integer is begin return n; end function;
  end package body;
begin
end;
