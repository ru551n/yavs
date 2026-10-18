package p is
  function f generic (type t) parameter (v : t) return integer;
  function g return integer is new f generic map (t => integer);
end package;
