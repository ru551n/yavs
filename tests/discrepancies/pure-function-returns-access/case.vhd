package p is
  type int_ptr is access integer;
  type rec is record a : int_ptr; end record;
  pure function f return int_ptr;
  pure function g return rec;
end package;

entity top is end;
architecture t of top is begin end;
