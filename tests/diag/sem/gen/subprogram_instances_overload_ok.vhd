-- yavs-test: std=2008
-- expect: none
-- LRM 4.4, 12.3: instances of one generic function declared under its own name overload by their
-- instantiated profiles, with an ordinary function of that name and across use clauses (#28)
package p is
  function to_s generic (type t; constant tag : string) parameter (v : t) return string;
end package;
package body p is
  function to_s generic (type t; constant tag : string) parameter (v : t) return string is
  begin
    return tag;
  end;
end package body;

use work.p.all;
package q is
  type rec is record a : integer; end record;
  type other is record b : boolean; end record;
  function to_s is new to_s generic map (t => rec, tag => "rec");
  function to_s is new to_s generic map (t => integer, tag => "int");
  function to_s(v : real) return string;
end package;
package body q is
  function to_s(v : real) return string is
  begin
    return "real";
  end;
end package body;

use work.p.all;
use work.q.all;
package r is
  function to_s is new to_s generic map (t => other, tag => "other");
  constant c1 : string := to_s(rec'(a => 1));
  constant c2 : string := to_s(5);
  constant c3 : string := to_s(2.5);
  constant c4 : string := to_s(other'(b => true));
end package;
