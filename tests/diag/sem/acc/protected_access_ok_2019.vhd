-- yavs-test: std=2019
-- expect: none
-- LRM 5.6.2 (2008): 2019 accepts an access result and a file parameter
package p is
  type int_ptr is access integer;
  type ft is file of integer;
  type box is protected
    impure function get return int_ptr;
    procedure put (file f : ft);
  end protected;
end package;
