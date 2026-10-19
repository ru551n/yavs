-- LRM 5.6.2: a protected method with a variable parameter of an access type.
package p is
  type int_ptr is access integer;
  type box is protected
    procedure put (variable x : inout int_ptr);
  end protected;
end package;
