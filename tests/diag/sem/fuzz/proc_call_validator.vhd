-- yavs-test: std=2008
-- expect: none
-- fuzz: a procedure call left an ExprKind::Error in the arena (validator ICE)
entity e is end;
architecture t of e is
  procedure p(x : integer) is begin null; end;
begin
  process begin
    p(2);
    wait;
  end process;
end;
