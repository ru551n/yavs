-- yavs-test: std=2008
-- expect: E1100 21:12 ua is not declared
-- expect: E1100 22:13 ub is not declared
-- expect: E1100 23:12 uc is not declared
-- expect: E1301 24:9 no function f matches the call
-- LRM 12.3, 12.5 (an actual or index with no interpretation of its own is reported, not the call or name around it)
entity e2 is end entity;
architecture a of e2 is
  type rec is record x : integer; end record;
  type recs is array (0 to 3) of rec;
  type ivs is array (natural range <>) of integer;
  function f (x : integer) return integer is begin return x; end function;
  function ff (x : integer) return ivs is begin return (0 to 1 => x); end function;
  procedure one (x : integer) is begin null; end procedure;
begin
  process
    variable i : integer;
    variable r : recs;
    variable a : ivs(0 to 3);
  begin
    i := r(ua(1)).x;
    i := ff(ub(1))(0);
    i := a(uc(1));
    one(f(1.0));
    wait;
  end process;
end architecture;
