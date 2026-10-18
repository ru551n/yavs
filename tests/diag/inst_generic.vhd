package p is
  procedure q generic (type t) parameter (v : t);
  procedure r generic (type u) is new q generic map (t => integer);
end package;
