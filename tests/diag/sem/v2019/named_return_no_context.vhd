-- yavs-test: std=2019
-- expect: E1304 19:35 `iota` has a named return value, but this context gives no constrained result subtype
-- help: assign the call to a constrained object
-- LRM 2019 4.2.1
entity e is
end entity;
architecture a of e is
  function iota return r of integer_vector is
    variable v : r;
  begin
    return v;
  end function;
  function first(x : integer_vector) return integer is
  begin
    return x(x'left);
  end function;
begin
  process
    variable n : natural := first(iota);
  begin
    wait;
  end process;
end architecture;
