-- yavs-test: std=2008
-- expect: E1100 11:26 `nosuch` is not declared
-- fuzz: a method whose profile failed is no subprogram: the call was an ICE (#535)
-- LRM 5.6.2
package p is
  procedure t;
end package;
package body p is
  procedure t is
    type pt is protected
      procedure set (x : nosuch);
    end protected;
    type pt is protected body
      procedure set (x : nosuch) is begin null; end procedure;
    end protected body;
    variable v : pt;
  begin
    v.set(true);
  end procedure;
end package body;
