-- yavs-test: std=2008
-- expect: E1301 13:19 the instance takes an integer, not a boolean
-- LRM 4.2.1: only the instantiated profile is visible, not the generic one
entity e is end;
architecture a of e is
  function f generic (type t) generic map (t => integer) parameter (v : t) return t is
  begin
    return v;
  end function;
begin
  process
  begin
    assert f(1) = f(true);
    wait;
  end process;
end;
