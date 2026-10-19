-- yavs-test: std=2008
-- expect: E1237 19:13 an instance in a subprogram whose library package body declares a shared variable
-- LRM 4.9, 4.3
package g is
  generic (n : integer);
  function f return integer;
end package;
package body g is
  type pt is protected
  end protected;
  type pt is protected body
  end protected body;
  shared variable v : pt;
  function f return integer is begin return n; end function;
end package body;
entity e is end;
architecture a of e is
  procedure p is
    package i is new work.g generic map (n => 1);
  begin
    null;
  end procedure;
begin
end;
