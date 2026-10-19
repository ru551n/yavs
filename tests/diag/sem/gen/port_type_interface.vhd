-- yavs-test: std=2008
-- expect: E1234 6:9 a type interface declaration is not a port
-- help: generic list
-- LRM 6.5.6.3: a port clause declares interface signals only
entity e is
  port (type t);
end entity;
