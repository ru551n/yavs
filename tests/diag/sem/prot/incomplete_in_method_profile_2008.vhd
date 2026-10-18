-- yavs-test: std=2008
-- expect: E1220 7:32 incomplete type `t` can only be used as an access designated type
-- LRM 5.4.2, ruling U-2 (sem.md 11): only library std may use incomplete types in method profiles
package p is
  type t;
  type pt is protected
    impure function get return t;
  end protected;
  type t is record
    a : integer;
  end record;
end package;
