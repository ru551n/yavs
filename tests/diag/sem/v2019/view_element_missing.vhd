-- yavs-test: std=2019
-- expect: E1901 9:8 element `b` of record `rec` has no mode in view `v`
-- LRM 2019 6.5.2
package p is
  type rec is record
    a : bit;
    b : bit;
  end record;
  view v of rec is
    a : in;
  end view;
end package;
