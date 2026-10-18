-- yavs-test: std=2019
-- expect: none
-- LRM 2019 6.5.2
package p is
  type rec is record
    a : bit;
    b : bit;
  end record;
  view v of rec is
    a : in;
    b : out;
  end view;
  alias vc is v'converse;
end package;
