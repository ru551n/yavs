-- yavs-test: std=2008
-- expect: E1100 16:13 ua is not declared
-- expect: E1100 17:13 ub is not declared
-- expect: E1100 18:12 uc is not declared
-- LRM 12.3, 16.2 (an undeclared index in the prefix of a predefined attribute is reported, not the attribute)
entity e4 is end entity;
architecture a of e4 is
  type ivs is array (natural range <>) of integer;
  signal ff : bit_vector(0 to 3);
  function f (x : integer) return ivs is begin return (0 to 1 => x); end function;
begin
  process
    variable i : integer;
    variable w : ivs(0 to 3);
  begin
    i := ff(ua)'length;
    i := ff(ub)'left;
    i := w(uc)'length;
    wait;
  end process;
end architecture;
