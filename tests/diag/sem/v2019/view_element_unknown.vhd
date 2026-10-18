-- yavs-test: std=2019
-- expect: E1901 10:5 record `rec` has no element `zz`
-- LRM 2019 6.5.2
package p is
  type rec is record
    a : bit;
  end record;
  view v of rec is
    a : in;
    zz : out;
  end view;
end package;
