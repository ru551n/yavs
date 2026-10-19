-- yavs-test: std=2019
-- expect: E1235 6:12 a formal generic subprogram has no generic list or generic map
-- help: only the profile
-- LRM 6.5.4: an interface subprogram has no generic header
entity e is
  generic (function f generic (type t) parameter (x : t) return t);
end entity;
