-- yavs-test: std=2008
-- expect: E1230 8:15 procedure `add` has no body
-- LRM 4.2.1, 4.3: a header-instance spec in a process without its body (#1136)
entity e is end;
architecture a of e is
begin
  process
    procedure add generic (constant n : integer) generic map (n => 7)
      parameter (variable x : inout integer);
  begin
    wait;
  end process;
end;
