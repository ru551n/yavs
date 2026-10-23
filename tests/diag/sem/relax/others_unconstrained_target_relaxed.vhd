-- yavs-test: std=2008 flags=--relaxed
-- expect: W1325 8:8 `others` takes the bounds of an unconstrained target
-- help: constrain the target's subtype
-- LRM 9.3.3.3
entity e is port (o : out bit_vector); end entity;
architecture a of e is
begin
  o <= (others => '1');
end architecture;
