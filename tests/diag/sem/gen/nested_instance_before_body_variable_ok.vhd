-- yavs-test: std=2008
-- expect: none
-- LRM 4.9
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
    constant k : integer := n;
    function f return integer is begin return k; end function;
  end package body;
begin
end;
