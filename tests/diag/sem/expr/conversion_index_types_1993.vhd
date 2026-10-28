-- yavs-test: std=1993
-- expect: E1350 18:10 cannot convert `century` to `millenia`
-- expect: E1350 19:10 cannot convert `ints` to `century`
-- LRM 1993 7.3.5: array types are closely related when their index types are, and their element types are the same (#813)
entity e is
end entity;
architecture a of e is
begin
  process
    type century is array (1 to 2) of real;
    type millenia is array (bit'('0') to bit'('1')) of real;
    type ints is array (1 to 2) of integer;
    type reals is array (3 to 4) of real;
    variable h : century;
    variable t : millenia;
    variable r : reals;
  begin
    t := millenia(h);
    h := century(ints'(1, 2));
    r := reals(h);
    wait;
  end process;
end architecture;
