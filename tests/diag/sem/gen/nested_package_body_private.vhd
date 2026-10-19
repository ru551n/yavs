-- yavs-test: std=2008
-- expect: E1100 12:29 `hidden` is declared only in the nested package body
-- LRM 4.8, 12.3
entity e is end;
architecture a of e is
  package p is
    constant c : integer := 1;
  end package;
  package body p is
    constant hidden : integer := 2;
  end package body;
  constant k : integer := p.hidden;
begin
end;
