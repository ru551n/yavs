-- rsim-test: std=2008
-- expect: none
-- LRM 5.4.2, ruling U-2 (sem.md 11): incomplete types in protected method profiles
package p is
  type t;
  type pt is protected
    impure function get return t;
    procedure put (v : t);
  end protected;
  type t is record
    a : integer;
  end record;
end package;
