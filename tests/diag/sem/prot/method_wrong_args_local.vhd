-- yavs-test: std=2008
-- expect: E1301 18:7 no method `set` matches the call
-- expect: E1100 19:12 protected type `pt` has no method `reset`
-- LRM 5.6.2
package p is
  procedure t;
end package;
package body p is
  procedure t is
    type pt is protected
      procedure set (x : integer);
    end protected;
    type pt is protected body
      procedure set (x : integer) is begin null; end procedure;
    end protected body;
    variable v : pt;
  begin
    v.set(true);
    v := v.reset;
  end procedure;
end package body;
