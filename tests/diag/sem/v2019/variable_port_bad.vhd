-- yavs-test: std=2019
-- expect: E1360 8:20 not a protected type
-- expect: E1360 9:20 not mode inout
-- LRM 6.5.2, 6.5.7.1 (2019): a variable port is of a protected type, mode inout
package pk is
  type prot_t is protected procedure p; end protected;
end package;
entity e1 is port (variable x : inout integer); end entity;
entity e2 is port (variable x : in work.pk.prot_t); end entity;
