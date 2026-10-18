package p is
  function f generic (type t) parameter (v : t) return integer;
  impure function g is new f generic map (t => integer);
end package;
