-- yavs-test: std=2019
-- expect: E1901 10:14 view `vo` is for `other`, element `i` has type `inner`
-- LRM 2019 6.5.2
package p is
  type inner is record x : bit; end record;
  type other is record y : bit; end record;
  type outer is record i : inner; end record;
  view vo of other is y : in; end view;
  view v of outer is
    i : view vo;
  end view;
end package;
