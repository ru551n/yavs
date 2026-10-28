-- yavs-test: std=2008
-- expect: E1100 10:24 record type `rec` has no element `valu`
-- help: an element with a similar name exists: `value`
-- LRM 9.3.3.3
package p is
  type rec is record
    value : integer;
    valid : boolean;
  end record;
  constant c : rec := (valu => 1, valid => true);
end package;
