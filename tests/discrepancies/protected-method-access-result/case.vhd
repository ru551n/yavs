-- LRM 5.6.2: a protected method whose result is of an access type.
package p is
  type int_ptr is access integer;
  type box is protected
    impure function get return int_ptr;
  end protected;
end package;
