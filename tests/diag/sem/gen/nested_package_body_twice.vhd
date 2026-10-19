-- yavs-test: std=2008
-- expect: E1101 10:16 second body of package `p`
-- LRM 4.8
entity e is end;
architecture a of e is
  package p is
  end package;
  package body p is
  end package body;
  package body p is
  end package body;
begin
end;
