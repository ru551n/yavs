-- yavs-test: std=2008
-- expect: E1360 14:10 cannot read `'stable` of port `pt` of mode `linkage`
-- expect: E1360 15:10 cannot read `'last_value` of port `pt` of mode `linkage`
-- LRM 6.5.2: a `linkage` port is read only as the actual of a `linkage` formal, not through an attribute of its value (#813)
entity e is
  port (pt : linkage boolean; pv : linkage bit_vector(0 to 3));
end entity;
architecture a of e is
begin
  process
    variable b : boolean;
    variable n : integer;
  begin
    b := pt'stable;
    b := pt'last_value;
    n := pv'length;
    wait;
  end process;
end architecture;
