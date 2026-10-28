-- yavs-test: std=2008
-- expect: E1100 14:36 `rec` has no element `valu`
-- help: an element with a similar name exists: `value`
-- LRM 8.3
package p is
  type rec is record
    value : integer;
    valid : boolean;
  end record;
  signal r : rec;
  signal v : integer;
end package;
package q is
  constant c : integer := work.p.r.valu;
end package;
