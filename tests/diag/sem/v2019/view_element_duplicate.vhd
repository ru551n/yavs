-- yavs-test: std=2019
-- expect: E1901 12:5 element `a` appears twice in view `v`
-- LRM 2019 6.5.2
package p is
  type rec is record
    a : bit;
    b : bit;
  end record;
  view v of rec is
    a : in;
    b : out;
    a : out;
  end view;
end package;
