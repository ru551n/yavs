-- yavs-test: std=2008
-- expect: none
-- LRM 5.4.2
package p is
  type node;
  type ptr is access node;
  type node is record
    next_n : ptr;
    v : integer;
  end record;
end package;
